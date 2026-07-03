//! Cliente HTTP genérico contra el API v4 de Cloudflare.
//!
//! Decodifica el envelope `CfResponse<T>`, aplica backoff ante 429 (rate limit
//! global: 1200 req / 5 min por usuario) y expone helpers `get`/`post`/etc.
//! Diseñado para pegarle a CUALQUIER endpoint sin depender del crate `cloudflare`.

use std::time::Duration;

use color_eyre::eyre::{bail, eyre, Result};
use reqwest::{Client, Method, Response, StatusCode};
use serde::de::DeserializeOwned;
use serde::Serialize;

use crate::model::CfResponse;

const BASE: &str = "https://api.cloudflare.com/client/v4";
const MAX_RETRIES: u32 = 3;

/// Cliente autenticado con un API token (Bearer). Clonable y barato de copiar.
#[derive(Clone)]
pub struct CfClient {
    http: Client,
    token: String,
}

impl CfClient {
    pub fn new(token: String) -> Result<Self> {
        let http = Client::builder()
            .user_agent(concat!("lazycf/", env!("CARGO_PKG_VERSION")))
            .timeout(Duration::from_secs(30))
            .build()?;
        Ok(Self { http, token })
    }

    /// Ejecuta la petición con backoff ante 429 y devuelve `(status, body)`.
    async fn raw<B>(&self, method: Method, path: &str, body: Option<&B>) -> Result<(StatusCode, String)>
    where
        B: Serialize + ?Sized,
    {
        let url = format!("{BASE}{path}");
        let mut attempt = 0u32;
        loop {
            attempt += 1;
            let mut req = self
                .http
                .request(method.clone(), &url)
                .bearer_auth(&self.token);
            if let Some(b) = body {
                req = req.json(b);
            }

            let resp = req.send().await?;
            let status = resp.status();

            // Rate limit: respeta Retry-After o backoff exponencial.
            if status == StatusCode::TOO_MANY_REQUESTS && attempt <= MAX_RETRIES {
                let wait = backoff_secs(&resp, attempt);
                tokio::time::sleep(Duration::from_secs(wait)).await;
                continue;
            }

            return Ok((status, resp.text().await?));
        }
    }

    /// Decodifica el envelope y falla si `success == false`.
    fn check<T: DeserializeOwned>(path: &str, status: StatusCode, text: &str) -> Result<CfResponse<T>> {
        let parsed: CfResponse<T> = serde_json::from_str(text)
            .map_err(|e| eyre!("decodificando {path}: {e}; cuerpo: {}", truncate(text, 300)))?;
        if !parsed.success {
            let msg = parsed
                .errors
                .iter()
                .map(|e| e.message.trim())
                .filter(|m| !m.is_empty())
                .collect::<Vec<_>>()
                .join("; ");
            bail!("{}", if msg.is_empty() { format!("Error HTTP {status}") } else { msg });
        }
        Ok(parsed)
    }

    /// Llamada genérica que devuelve `result`. `body` se envía como JSON si es `Some`.
    pub async fn request<T, B>(&self, method: Method, path: &str, body: Option<&B>) -> Result<T>
    where
        T: DeserializeOwned,
        B: Serialize + ?Sized,
    {
        let (status, text) = self.raw(method, path, body).await?;
        let parsed = Self::check::<T>(path, status, &text)?;
        parsed
            .result
            .ok_or_else(|| eyre!("resultado vacío para {path}"))
    }

    /// Como `request` pero ignora `result` (para endpoints que devuelven `null`).
    pub async fn send_ok<B>(&self, method: Method, path: &str, body: Option<&B>) -> Result<()>
    where
        B: Serialize + ?Sized,
    {
        let (status, text) = self.raw(method, path, body).await?;
        Self::check::<serde_json::Value>(path, status, &text)?;
        Ok(())
    }

    /// `DELETE` que solo comprueba éxito (ignora el cuerpo del resultado).
    pub async fn delete_ok(&self, path: &str) -> Result<()> {
        self.send_ok::<()>(Method::DELETE, path, None).await
    }

    /// GET que devuelve el envelope completo como JSON (para endpoints con
    /// `result_info` extendido, p. ej. objetos R2 con `delimited`/`cursor`).
    pub async fn get_value(&self, path: &str) -> Result<serde_json::Value> {
        let (status, text) = self.raw::<()>(Method::GET, path, None).await?;
        let v: serde_json::Value = serde_json::from_str(&text)
            .map_err(|e| eyre!("decodificando {path}: {e}; cuerpo: {}", truncate(&text, 300)))?;
        if !v["success"].as_bool().unwrap_or(false) {
            let msg = v["errors"]
                .as_array()
                .map(|errs| {
                    errs.iter()
                        .filter_map(|e| e["message"].as_str())
                        .map(str::trim)
                        .filter(|m| !m.is_empty())
                        .collect::<Vec<_>>()
                        .join("; ")
                })
                .unwrap_or_default();
            bail!("{}", if msg.is_empty() { format!("Error HTTP {status}") } else { msg });
        }
        Ok(v)
    }

    /// GET que devuelve el cuerpo binario tal cual (descarga de objetos R2).
    /// En error el cuerpo es el envelope JSON → se extrae el mensaje.
    pub async fn get_bytes(&self, path: &str) -> Result<Vec<u8>> {
        let url = format!("{BASE}{path}");
        let resp = self.http.get(&url).bearer_auth(&self.token).send().await?;
        let status = resp.status();
        let bytes = resp.bytes().await?;
        if !status.is_success() {
            let text = String::from_utf8_lossy(&bytes).to_string();
            Self::check::<serde_json::Value>(path, status, &text)?;
            bail!("Error HTTP {status}");
        }
        Ok(bytes.to_vec())
    }

    /// PUT con cuerpo binario (subida de objetos R2); la respuesta es envelope.
    pub async fn put_bytes(&self, path: &str, body: Vec<u8>, content_type: &str) -> Result<()> {
        let url = format!("{BASE}{path}");
        let resp = self
            .http
            .put(&url)
            .bearer_auth(&self.token)
            .header(reqwest::header::CONTENT_TYPE, content_type)
            .body(body)
            .send()
            .await?;
        let status = resp.status();
        let text = resp.text().await?;
        Self::check::<serde_json::Value>(path, status, &text)?;
        Ok(())
    }

    /// Envía un formulario `multipart/form-data` y solo comprueba éxito.
    /// Sin backoff de 429 (mutaciones puntuales); reutiliza el decoder de envelope.
    pub async fn multipart_ok(
        &self,
        method: Method,
        path: &str,
        form: reqwest::multipart::Form,
    ) -> Result<()> {
        let url = format!("{BASE}{path}");
        let resp = self
            .http
            .request(method, &url)
            .bearer_auth(&self.token)
            .multipart(form)
            .send()
            .await?;
        let status = resp.status();
        let text = resp.text().await?;
        Self::check::<serde_json::Value>(path, status, &text)?;
        Ok(())
    }

    /// Consulta GraphQL (`/graphql`). Devuelve el campo `data` deserializado.
    pub async fn graphql<T: DeserializeOwned>(
        &self,
        query: &str,
        variables: serde_json::Value,
    ) -> Result<T> {
        #[derive(serde::Deserialize)]
        struct GqlResp<T> {
            data: Option<T>,
            // GraphQL devuelve `errors: null` cuando no hay errores (no `[]`),
            // por eso es Option y no `Vec` con default.
            #[serde(default)]
            errors: Option<Vec<GqlErr>>,
        }
        #[derive(serde::Deserialize)]
        struct GqlErr {
            message: String,
        }

        let body = serde_json::json!({ "query": query, "variables": variables });
        let (_status, text) = self.raw(Method::POST, "/graphql", Some(&body)).await?;
        let parsed: GqlResp<T> = serde_json::from_str(&text)
            .map_err(|e| eyre!("decodificando GraphQL: {e}; cuerpo: {}", truncate(&text, 300)))?;
        let errors = parsed.errors.unwrap_or_default();
        if !errors.is_empty() {
            let msg = errors
                .iter()
                .map(|e| e.message.trim())
                .filter(|m| !m.is_empty())
                .collect::<Vec<_>>()
                .join("; ");
            bail!("{}", if msg.is_empty() { "error GraphQL".into() } else { msg });
        }
        parsed.data.ok_or_else(|| eyre!("GraphQL sin datos"))
    }

    pub async fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T> {
        self.request::<T, ()>(Method::GET, path, None).await
    }

    #[allow(dead_code)] // usado a partir de Fase 1 (crear/editar recursos)
    pub async fn post<T: DeserializeOwned, B: Serialize + ?Sized>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<T> {
        self.request(Method::POST, path, Some(body)).await
    }

    #[allow(dead_code)] // usado a partir de Fase 1 (editar recursos)
    pub async fn patch<T: DeserializeOwned, B: Serialize + ?Sized>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<T> {
        self.request(Method::PATCH, path, Some(body)).await
    }

    pub async fn put<T: DeserializeOwned, B: Serialize + ?Sized>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<T> {
        self.request(Method::PUT, path, Some(body)).await
    }

    #[allow(dead_code)] // usado a partir de Fase 1 (borrar recursos)
    pub async fn delete<T: DeserializeOwned>(&self, path: &str) -> Result<T> {
        self.request::<T, ()>(Method::DELETE, path, None).await
    }
}

/// Segundos a esperar tras un 429: header `Retry-After` o backoff exponencial.
fn backoff_secs(resp: &Response, attempt: u32) -> u64 {
    resp.headers()
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or_else(|| 2u64.saturating_pow(attempt))
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}…", &s[..max])
    }
}
