//! Endpoints de Workers (Fase 3): listar scripts, subdominio workers.dev,
//! métricas vía GraphQL y una sonda HTTP para probar rutas.

use std::time::{Duration, Instant};

use chrono::DateTime;
use color_eyre::eyre::Result;
use reqwest::Method;
use serde::Deserialize;
use serde_json::json;
use tokio::net::TcpStream;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

use super::CfClient;
use crate::model::{Binding, Deployment, WorkerMetrics, WorkerScript};

/// Stream WebSocket ya conectado para el live-tail (`trace-v1`).
pub type TailWs = WebSocketStream<MaybeTlsStream<TcpStream>>;

const METRICS_QUERY: &str = r#"
query($accountTag: string!, $scriptName: string!, $start: string!, $end: string!) {
  viewer {
    accounts(filter: { accountTag: $accountTag }) {
      workersInvocationsAdaptive(
        limit: 100,
        filter: { scriptName: $scriptName, datetime_geq: $start, datetime_leq: $end },
        orderBy: [datetimeHour_ASC]
      ) {
        sum { requests errors }
        quantiles { cpuTimeP50 cpuTimeP99 }
        dimensions { datetimeHour }
      }
    }
  }
}
"#;

// --- Estructuras de la respuesta GraphQL ---

#[derive(Debug, Deserialize)]
struct GqlData {
    viewer: Viewer,
}
#[derive(Debug, Deserialize)]
struct Viewer {
    accounts: Vec<AccountBlock>,
}
#[derive(Debug, Deserialize)]
struct AccountBlock {
    // CF puede devolver `null` (no solo ausente) si no hay datos en la ventana.
    #[serde(rename = "workersInvocationsAdaptive", default)]
    groups: Option<Vec<InvocationGroup>>,
}
#[derive(Debug, Deserialize)]
struct InvocationGroup {
    sum: GroupSum,
    #[serde(default)]
    quantiles: Option<GroupQuantiles>,
}
#[derive(Debug, Deserialize, Default)]
struct GroupSum {
    #[serde(default)]
    requests: u64,
    #[serde(default)]
    errors: u64,
}
#[derive(Debug, Deserialize, Default)]
struct GroupQuantiles {
    #[serde(rename = "cpuTimeP50", default)]
    cpu_p50: f64,
    #[serde(rename = "cpuTimeP99", default)]
    cpu_p99: f64,
}

impl CfClient {
    /// `GET /accounts/{id}/workers/scripts`.
    pub async fn list_scripts(&self, account_id: &str) -> Result<Vec<WorkerScript>> {
        self.get(&format!("/accounts/{account_id}/workers/scripts"))
            .await
    }

    /// `GET /accounts/{id}/workers/subdomain` → subdominio `*.workers.dev`.
    pub async fn workers_subdomain(&self, account_id: &str) -> Result<Option<String>> {
        #[derive(Deserialize)]
        struct Sub {
            #[serde(default)]
            subdomain: String,
        }
        let s: Sub = self
            .get(&format!("/accounts/{account_id}/workers/subdomain"))
            .await?;
        Ok((!s.subdomain.is_empty()).then_some(s.subdomain))
    }

    /// Métricas del Worker en `[start, end]` (RFC3339) vía GraphQL.
    pub async fn worker_metrics(
        &self,
        account_id: &str,
        script: &str,
        start: &str,
        end: &str,
    ) -> Result<WorkerMetrics> {
        let vars = json!({
            "accountTag": account_id,
            "scriptName": script,
            "start": start,
            "end": end,
        });
        let data: GqlData = self.graphql(METRICS_QUERY, vars).await?;
        let mut m = WorkerMetrics::default();
        if let Some(account) = data.viewer.accounts.into_iter().next() {
            for g in account.groups.unwrap_or_default() {
                m.requests += g.sum.requests;
                m.errors += g.sum.errors;
                m.series.push(g.sum.requests); // ya viene ordenado por hora
                if let Some(q) = g.quantiles {
                    // Toma los peores percentiles observados.
                    m.cpu_p50 = m.cpu_p50.max(q.cpu_p50);
                    m.cpu_p99 = m.cpu_p99.max(q.cpu_p99);
                }
            }
        }
        Ok(m)
    }

    /// `GET /accounts/{id}/workers/scripts/{s}/deployments`.
    pub async fn list_deployments(
        &self,
        account_id: &str,
        script: &str,
    ) -> Result<Vec<Deployment>> {
        #[derive(Deserialize)]
        struct Resp {
            #[serde(default)]
            deployments: Vec<Deployment>,
        }
        let r: Resp = self
            .get(&format!(
                "/accounts/{account_id}/workers/scripts/{script}/deployments"
            ))
            .await?;
        Ok(r.deployments)
    }

    /// `GET /accounts/{id}/workers/scripts/{s}/settings` → bindings (vars/secretos/…).
    pub async fn worker_bindings(&self, account_id: &str, script: &str) -> Result<Vec<Binding>> {
        #[derive(Deserialize)]
        struct Settings {
            #[serde(default)]
            bindings: Vec<Binding>,
        }
        let s: Settings = self
            .get(&format!(
                "/accounts/{account_id}/workers/scripts/{script}/settings"
            ))
            .await?;
        Ok(s.bindings)
    }

    /// Crea/actualiza un secreto (`PUT .../secrets`). Endpoint aislado: no toca
    /// el resto de bindings, así que es seguro para producción.
    pub async fn put_secret(
        &self,
        account_id: &str,
        script: &str,
        name: &str,
        text: &str,
    ) -> Result<()> {
        let body = json!({ "name": name, "text": text, "type": "secret_text" });
        self.send_ok(
            Method::PUT,
            &format!("/accounts/{account_id}/workers/scripts/{script}/secrets"),
            Some(&body),
        )
        .await
    }

    /// Actualiza los bindings del Worker vía `PATCH .../settings` (multipart).
    /// El caller pasa la lista completa; los bindings a conservar sin cambios se
    /// envían como `{"type":"inherit","name":…}` para no perder secretos ni otros.
    pub async fn update_worker_bindings(
        &self,
        account_id: &str,
        script: &str,
        bindings: serde_json::Value,
    ) -> Result<()> {
        let settings = json!({ "bindings": bindings }).to_string();
        let form = reqwest::multipart::Form::new().text("settings", settings);
        self.multipart_ok(
            Method::PATCH,
            &format!("/accounts/{account_id}/workers/scripts/{script}/settings"),
            form,
        )
        .await
    }

    /// Abre una sesión de tail (`POST .../tails`). Devuelve `(id, wss_url)`.
    /// El túnel WebSocket ya viene autenticado en la URL (no lleva Bearer).
    pub async fn create_tail(&self, account_id: &str, script: &str) -> Result<(String, String)> {
        #[derive(Deserialize)]
        struct TailInfo {
            #[serde(default)]
            id: String,
            #[serde(default)]
            url: String,
        }
        // Los filtros van en el body del POST (como wrangler); vacío = todo.
        let t: TailInfo = self
            .post(
                &format!("/accounts/{account_id}/workers/scripts/{script}/tails"),
                &json!({ "filters": [] }),
            )
            .await?;
        Ok((t.id, t.url))
    }

    /// Cierra una sesión de tail (`DELETE .../tails/{id}`).
    pub async fn delete_tail(&self, account_id: &str, script: &str, tail_id: &str) -> Result<()> {
        self.delete_ok(&format!(
            "/accounts/{account_id}/workers/scripts/{script}/tails/{tail_id}"
        ))
        .await
    }
}

/// Conecta al WebSocket de tail con el subprotocolo `trace-v1` que exige Cloudflare.
pub async fn connect_tail(url: &str) -> Result<TailWs> {
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;
    use tokio_tungstenite::tungstenite::http::HeaderValue;

    let mut req = url.into_client_request()?;
    req.headers_mut().insert(
        "sec-websocket-protocol",
        HeaderValue::from_static("trace-v1"),
    );
    let (ws, _resp) = tokio_tungstenite::connect_async(req).await?;
    Ok(ws)
}

/// Convierte un mensaje de tail (JSON) en líneas legibles para el panel.
pub fn parse_tail(raw: &str) -> Vec<String> {
    #[derive(Deserialize)]
    struct Ev {
        #[serde(default)]
        outcome: String,
        #[serde(default)]
        logs: Vec<Lg>,
        #[serde(default)]
        exceptions: Vec<Ex>,
        #[serde(default, rename = "eventTimestamp")]
        ts: Option<i64>,
        #[serde(default)]
        event: Option<serde_json::Value>,
    }
    #[derive(Deserialize)]
    struct Lg {
        #[serde(default)]
        level: String,
        #[serde(default)]
        message: Vec<serde_json::Value>,
        #[serde(default)]
        timestamp: Option<i64>,
    }
    #[derive(Deserialize)]
    struct Ex {
        #[serde(default)]
        name: String,
        #[serde(default)]
        message: String,
    }

    let Ok(ev) = serde_json::from_str::<Ev>(raw) else {
        tracing::debug!("mensaje de tail no reconocido: {raw}");
        return Vec::new();
    };
    let hhmmss = |ms: Option<i64>| {
        ms.and_then(DateTime::from_timestamp_millis)
            .map(|d| d.format("%H:%M:%S").to_string())
            .unwrap_or_default()
    };

    let mut out = Vec::new();
    let trigger = ev.event.as_ref().map(describe_event).unwrap_or_default();
    let mut head = format!("▪ {}", hhmmss(ev.ts));
    if !trigger.is_empty() {
        head.push(' ');
        head.push_str(&trigger);
    }
    if !ev.outcome.is_empty() {
        head.push_str(&format!("  · {}", ev.outcome));
    }
    out.push(head);

    for l in &ev.logs {
        let lvl = if l.level.is_empty() {
            "log".to_string()
        } else {
            l.level.clone()
        };
        out.push(format!(
            "  {} [{}] {}",
            hhmmss(l.timestamp),
            lvl,
            join_message(&l.message)
        ));
    }
    for e in &ev.exceptions {
        out.push(format!("  ✗ {}: {}", e.name, e.message));
    }
    out
}

/// Describe el disparador de un evento de tail (request / cron / queue).
fn describe_event(v: &serde_json::Value) -> String {
    if let Some(req) = v.get("request") {
        let method = req.get("method").and_then(|m| m.as_str()).unwrap_or("GET");
        let url = req.get("url").and_then(|u| u.as_str()).unwrap_or("");
        return format!("{method} {url}");
    }
    if let Some(cron) = v.get("cron").and_then(|c| c.as_str()) {
        return format!("cron {cron}");
    }
    if let Some(q) = v.get("queue").and_then(|q| q.as_str()) {
        return format!("queue {q}");
    }
    if v.get("scheduledTime").is_some() {
        return "scheduled".to_string();
    }
    String::new()
}

/// Une los argumentos de `console.log` en una sola línea.
fn join_message(items: &[serde_json::Value]) -> String {
    items
        .iter()
        .map(|v| match v {
            serde_json::Value::String(s) => s.clone(),
            other => other.to_string(),
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Resultado de una sonda HTTP a una ruta de Worker.
pub struct ProbeResult {
    pub status: Option<u16>,
    pub millis: u128,
    pub info: String,
}

/// Hace un GET a `url` (sin auth) y mide latencia. No usa el API de Cloudflare.
pub async fn http_probe(url: String) -> ProbeResult {
    let client = match reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            return ProbeResult {
                status: None,
                millis: 0,
                info: e.to_string(),
            };
        }
    };
    let start = Instant::now();
    match client.get(&url).send().await {
        Ok(resp) => {
            let status = resp.status();
            let millis = start.elapsed().as_millis();
            let body = resp.text().await.unwrap_or_default();
            let snippet: String = body.chars().take(160).collect();
            ProbeResult {
                status: Some(status.as_u16()),
                millis,
                info: snippet,
            }
        }
        Err(e) => ProbeResult {
            status: None,
            millis: start.elapsed().as_millis(),
            info: e.to_string(),
        },
    }
}
