//! Cliente HTTP genérico contra el API v4 de Cloudflare.
//!
//! Decodifica el envelope `CfResponse<T>`, aplica backoff ante 429 (rate limit
//! global: 1200 req / 5 min por usuario) y expone helpers `get`/`post`/etc.
//! Diseñado para pegarle a CUALQUIER endpoint sin depender del crate `cloudflare`.
//!
//! La credencial vive en un [`CredentialSource`] compartido: para OAuth el
//! access token se refresca in situ (proactivo y ante 401) con single-flight,
//! porque Cloudflare rota el `refresh_token` en cada uso y dos refresh
//! concurrentes matarían la sesión.

use std::sync::{Arc, RwLock};
use std::time::Duration;

use color_eyre::eyre::{Result, bail, eyre};
use reqwest::{Client, Method, Response, StatusCode};
use serde::Serialize;
use serde::de::DeserializeOwned;
use tokio::sync::mpsc::UnboundedSender;

use crate::action::Action;
use crate::model::CfResponse;
use crate::oauth;
use crate::secrets::Credential;

const BASE: &str = "https://api.cloudflare.com/client/v4";
const MAX_RETRIES: u32 = 3;
/// Margen antes de `expires_at` para refrescar proactivamente (clock skew y
/// requests en vuelo).
const REFRESH_MARGIN_SECS: i64 = 60;

/// Origen de credencial compartido entre todos los clones de un `CfClient` y
/// su `Session`: fuente única de verdad, para que un refresh interno nunca
/// deje copias stale (que persistirían un refresh_token ya rotado).
pub struct CredentialSource {
    cred: RwLock<Credential>,
    /// Single-flight: solo un refresh a la vez entre tareas concurrentes.
    refresh_lock: tokio::sync::Mutex<()>,
    /// Notifica al App tras cada refresh para que persista la lista completa.
    on_refresh: Option<UnboundedSender<Action>>,
    client_id: String,
    /// Override del token endpoint (solo tests); `None` = endpoint real.
    token_url: Option<String>,
}

impl CredentialSource {
    pub fn new(cred: Credential, on_refresh: Option<UnboundedSender<Action>>) -> Arc<Self> {
        Arc::new(Self {
            cred: RwLock::new(cred),
            refresh_lock: tokio::sync::Mutex::new(()),
            on_refresh,
            client_id: oauth::client_id(),
            token_url: None,
        })
    }

    /// Copia de la credencial vigente (siempre fresca tras un refresh).
    pub fn credential(&self) -> Credential {
        self.cred.read().unwrap().clone()
    }

    pub fn is_oauth(&self) -> bool {
        self.credential().is_oauth()
    }

    /// Access token / API token vigente para `bearer_auth`.
    fn access_token(&self) -> String {
        match &*self.cred.read().unwrap() {
            Credential::ApiToken { token } => token.clone(),
            Credential::OAuth { tokens } => tokens.access_token.clone(),
        }
    }

    /// `true` si es OAuth y el access token expiró o expira en breve.
    fn needs_refresh(&self) -> bool {
        match &*self.cred.read().unwrap() {
            Credential::ApiToken { .. } => false,
            Credential::OAuth { tokens } => tokens.expires_within(REFRESH_MARGIN_SECS),
        }
    }
}

/// Cliente autenticado (Bearer). Clonable y barato de copiar; todos los
/// clones comparten el mismo `CredentialSource`.
#[derive(Clone)]
pub struct CfClient {
    http: Client,
    source: Arc<CredentialSource>,
}

impl CfClient {
    /// Cliente sobre un origen de credencial compartido (OAuth o API token).
    pub fn from_source(source: Arc<CredentialSource>) -> Result<Self> {
        let http = Client::builder()
            .user_agent(concat!("lazycf/", env!("CARGO_PKG_VERSION")))
            .timeout(Duration::from_secs(30))
            .build()?;
        Ok(Self { http, source })
    }

    pub fn source(&self) -> &Arc<CredentialSource> {
        &self.source
    }

    /// Token vigente para la request; refresca proactivamente si va a expirar.
    async fn bearer(&self) -> Result<String> {
        if self.source.needs_refresh() {
            let stale = self.source.access_token();
            self.refresh_credential(&stale).await?;
        }
        Ok(self.source.access_token())
    }

    /// Refresca la credencial OAuth con single-flight:
    /// 1. Adquiere el lock de refresh (una tarea a la vez).
    /// 2. Re-lee la credencial: si el access token ya cambió (otra tarea
    ///    refrescó mientras esperábamos) no hace nada.
    /// 3. Si no cambió, refresca contra el token endpoint, escribe el
    ///    resultado y notifica al App para que persista.
    ///
    /// No-op para API tokens.
    async fn refresh_credential(&self, stale_access: &str) -> Result<()> {
        let _guard = self.source.refresh_lock.lock().await;
        let Credential::OAuth { tokens } = self.source.credential() else {
            return Ok(());
        };
        if tokens.access_token != stale_access {
            return Ok(()); // otra tarea ya refrescó: reutilizar el token nuevo
        }
        let new_tokens = match &self.source.token_url {
            Some(url) => {
                oauth::refresh_at(url, &self.source.client_id, &tokens.refresh_token).await?
            }
            None => oauth::refresh(&self.source.client_id, &tokens.refresh_token).await?,
        };
        *self.source.cred.write().unwrap() = Credential::OAuth { tokens: new_tokens };
        if let Some(tx) = &self.source.on_refresh {
            let _ = tx.send(Action::CredentialRefreshed);
        }
        Ok(())
    }

    /// Ejecuta la petición con backoff ante 429 y devuelve `(status, body)`.
    /// Con credencial OAuth, un 401 dispara un refresh (single-flight) y un
    /// único reintento; si persiste, se devuelve el 401 tal cual (no loop).
    async fn raw<B>(
        &self,
        method: Method,
        path: &str,
        body: Option<&B>,
    ) -> Result<(StatusCode, String)>
    where
        B: Serialize + ?Sized,
    {
        let url = format!("{BASE}{path}");
        let mut attempt = 0u32;
        let mut refreshed = false;
        loop {
            attempt += 1;
            let token = self.bearer().await?;
            let mut req = self.http.request(method.clone(), &url).bearer_auth(&token);
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

            // Access token OAuth expirado: refrescar y reintentar una vez.
            if status == StatusCode::UNAUTHORIZED && !refreshed && self.source.is_oauth() {
                refreshed = true;
                self.refresh_credential(&token).await?;
                continue;
            }

            return Ok((status, resp.text().await?));
        }
    }

    /// Decodifica el envelope y falla si `success == false`.
    fn check<T: DeserializeOwned>(
        path: &str,
        status: StatusCode,
        text: &str,
    ) -> Result<CfResponse<T>> {
        let parsed: CfResponse<T> = serde_json::from_str(text).map_err(|e| {
            eyre!(
                "decodificando {path}: {e}; contexto: {}",
                error_context(text, e.line(), 400)
            )
        })?;
        if !parsed.success {
            let msg = parsed
                .errors
                .iter()
                .map(|e| e.message.trim())
                .filter(|m| !m.is_empty())
                .collect::<Vec<_>>()
                .join("; ");
            bail!(
                "{}",
                if msg.is_empty() {
                    format!("Error HTTP {status}")
                } else {
                    msg
                }
            );
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
        let v: serde_json::Value = serde_json::from_str(&text).map_err(|e| {
            eyre!(
                "decodificando {path}: {e}; cuerpo: {}",
                truncate(&text, 300)
            )
        })?;
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
            bail!(
                "{}",
                if msg.is_empty() {
                    format!("Error HTTP {status}")
                } else {
                    msg
                }
            );
        }
        Ok(v)
    }

    /// GET que devuelve el cuerpo binario tal cual (descarga de objetos R2).
    /// En error el cuerpo es el envelope JSON → se extrae el mensaje.
    pub async fn get_bytes(&self, path: &str) -> Result<Vec<u8>> {
        let url = format!("{BASE}{path}");
        let resp = self
            .http
            .get(&url)
            .bearer_auth(self.bearer().await?)
            .send()
            .await?;
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
            .bearer_auth(self.bearer().await?)
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
            .bearer_auth(self.bearer().await?)
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
        let parsed: GqlResp<T> = serde_json::from_str(&text).map_err(|e| {
            eyre!(
                "decodificando GraphQL: {e}; cuerpo: {}",
                truncate(&text, 300)
            )
        })?;
        let errors = parsed.errors.unwrap_or_default();
        if !errors.is_empty() {
            let msg = errors
                .iter()
                .map(|e| e.message.trim())
                .filter(|m| !m.is_empty())
                .collect::<Vec<_>>()
                .join("; ");
            bail!(
                "{}",
                if msg.is_empty() {
                    "error GraphQL".into()
                } else {
                    msg
                }
            );
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

/// Extracto del cuerpo alrededor de la línea donde falló el parseo (en vez de
/// solo el principio): mucho más útil para ver qué campo llegó `null`.
fn error_context(text: &str, line: usize, max: usize) -> String {
    // `line` de serde_json es 1-indexado; localiza el byte donde empieza.
    let target = line.saturating_sub(1);
    let mut offset = 0usize;
    for (i, l) in text.split('\n').enumerate() {
        if i == target {
            break;
        }
        offset += l.len() + 1;
    }
    let start = offset.saturating_sub(max / 2).min(text.len());
    let end = (offset + max / 2).min(text.len());
    // No cortar en medio de un carácter UTF-8 multibyte.
    let start = (start..=offset.min(text.len()))
        .find(|i| text.is_char_boundary(*i))
        .unwrap_or(0);
    let end = (end..=text.len())
        .find(|i| text.is_char_boundary(*i))
        .unwrap_or(text.len());
    let prefix = if start > 0 { "…" } else { "" };
    let suffix = if end < text.len() { "…" } else { "" };
    format!("{prefix}{}{suffix}", &text[start..end])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::oauth::OAuthTokens;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Servidor local que responde tokens nuevos y cuenta cuántos refresh recibe.
    async fn mock_token_server(hits: Arc<AtomicUsize>) -> String {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            loop {
                let Ok((mut stream, _)) = listener.accept().await else {
                    return;
                };
                let hits = hits.clone();
                tokio::spawn(async move {
                    let mut buf = vec![0u8; 8192];
                    let _ = stream.read(&mut buf).await;
                    let n = hits.fetch_add(1, Ordering::SeqCst) + 1;
                    let body = format!(
                        r#"{{"access_token":"at-nuevo-{n}","token_type":"bearer","expires_in":3600,"refresh_token":"rt-nuevo-{n}","scope":"s"}}"#
                    );
                    let resp = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    );
                    let _ = stream.write_all(resp.as_bytes()).await;
                });
            }
        });
        format!("http://127.0.0.1:{port}/oauth2/token")
    }

    #[tokio::test]
    async fn single_flight_refresca_exactamente_una_vez() {
        let hits = Arc::new(AtomicUsize::new(0));
        let token_url = mock_token_server(hits.clone()).await;

        // Credencial OAuth ya expirada → toda request necesita refresh.
        let expirada = Credential::OAuth {
            tokens: OAuthTokens {
                access_token: "at-viejo".into(),
                refresh_token: "rt-viejo".into(),
                expires_at: 0,
                scopes: "s".into(),
            },
        };
        let source = Arc::new(CredentialSource {
            cred: RwLock::new(expirada),
            refresh_lock: tokio::sync::Mutex::new(()),
            on_refresh: None,
            client_id: "cid".into(),
            token_url: Some(token_url),
        });
        let client = CfClient::from_source(source.clone()).unwrap();

        // N tareas concurrentes piden token con la credencial expirada.
        let mut handles = Vec::new();
        for _ in 0..8 {
            let c = client.clone();
            handles.push(tokio::spawn(async move { c.bearer().await.unwrap() }));
        }
        let mut tokens = Vec::new();
        for h in handles {
            tokens.push(h.await.unwrap());
        }

        // Exactamente UN refresh; todas las tareas reutilizan el token nuevo.
        assert_eq!(hits.load(Ordering::SeqCst), 1);
        assert!(tokens.iter().all(|t| t == "at-nuevo-1"), "{tokens:?}");
        match source.credential() {
            Credential::OAuth { tokens } => assert_eq!(tokens.refresh_token, "rt-nuevo-1"),
            _ => panic!("debe seguir siendo OAuth"),
        }
    }

    use super::error_context;

    #[test]
    fn contexto_ubica_la_linea_correcta() {
        let text = "{\n  \"a\": 1,\n  \"b\": null,\n  \"c\": 3\n}";
        let ctx = error_context(text, 3, 400);
        assert!(ctx.contains("\"b\": null"), "contexto: {ctx}");
    }

    #[test]
    fn contexto_recorta_y_marca_con_puntos_suspensivos() {
        let mut text = String::from("{\n");
        for i in 0..200 {
            text.push_str(&format!("  \"campo{i}\": {i},\n"));
        }
        text.push_str("  \"malo\": null\n}");
        let line = text.matches('\n').count(); // línea de "malo"
        let ctx = error_context(&text, line, 60);
        assert!(ctx.contains("malo"), "contexto: {ctx}");
        assert!(ctx.len() < text.len(), "debe recortar el cuerpo completo");
        assert!(ctx.starts_with('…'), "debe marcar el corte inicial: {ctx}");
    }
}
