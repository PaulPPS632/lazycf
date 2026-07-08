//! Endpoints de Queues (Fase 4): listar/crear/borrar colas, pausar entrega,
//! publicar/purgar/espiar mensajes, consumers y métricas GraphQL.

use color_eyre::eyre::Result;
use reqwest::Method;
use serde::Deserialize;
use serde_json::json;

use super::CfClient;
use crate::model::{PulledMessage, Queue, QueueConsumer, QueueMetrics};

/// RIESGO conocido: los nombres exactos del dataset/dimensiones GraphQL
/// (`queueId` vs `queueID`, sufijos `_geq`) se validan en runtime —
/// `client.graphql` hace bail con el error de GraphQL, que nombra los campos
/// válidos. Si las métricas fallan, ajustar SOLO esta constante.
const QUEUE_METRICS_QUERY: &str = r#"
query($accountTag: string!, $queueId: string!, $start: Time!, $end: Time!) {
  viewer {
    accounts(filter: { accountTag: $accountTag }) {
      queueBacklogAdaptiveGroups(
        limit: 100,
        filter: { queueId: $queueId, datetimeHour_geq: $start, datetimeHour_leq: $end },
        orderBy: [datetimeHour_ASC]
      ) {
        avg { messages bytes }
        dimensions { datetimeHour }
      }
      queueMessageOperationsAdaptiveGroups(
        limit: 100,
        filter: { queueId: $queueId, actionType: "WriteMessage",
                  datetimeHour_geq: $start, datetimeHour_leq: $end },
        orderBy: [datetimeHour_ASC]
      ) {
        count
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
    #[serde(rename = "queueBacklogAdaptiveGroups", default)]
    backlog: Option<Vec<BacklogGroup>>,
    #[serde(rename = "queueMessageOperationsAdaptiveGroups", default)]
    ops: Option<Vec<OpsGroup>>,
}
#[derive(Debug, Deserialize)]
struct BacklogGroup {
    #[serde(default)]
    avg: BacklogAvg,
}
#[derive(Debug, Deserialize, Default)]
struct BacklogAvg {
    #[serde(default)]
    messages: f64,
    #[serde(default)]
    bytes: f64,
}
#[derive(Debug, Deserialize)]
struct OpsGroup {
    #[serde(default)]
    count: u64,
}

impl CfClient {
    /// `GET /accounts/{id}/queues` — colas de la cuenta.
    pub async fn list_queues(&self, account_id: &str) -> Result<Vec<Queue>> {
        self.get(&format!("/accounts/{account_id}/queues")).await
    }

    /// `POST /accounts/{id}/queues` — crea una cola.
    pub async fn create_queue(&self, account_id: &str, name: &str) -> Result<()> {
        self.send_ok(
            Method::POST,
            &format!("/accounts/{account_id}/queues"),
            Some(&json!({ "queue_name": name })),
        )
        .await
    }

    /// `DELETE /queues/{id}` — borra la cola y sus mensajes pendientes.
    pub async fn delete_queue(&self, account_id: &str, queue_id: &str) -> Result<()> {
        self.delete_ok(&format!("/accounts/{account_id}/queues/{queue_id}"))
            .await
    }

    /// `PATCH /queues/{id}` — pausa/reanuda la entrega. Se envía también
    /// `queue_name` (mismo valor, no-op) por si el API lo exige en el body.
    pub async fn set_delivery_paused(
        &self,
        account_id: &str,
        queue_id: &str,
        queue_name: &str,
        paused: bool,
    ) -> Result<()> {
        self.send_ok(
            Method::PATCH,
            &format!("/accounts/{account_id}/queues/{queue_id}"),
            Some(&json!({
                "queue_name": queue_name,
                "settings": { "delivery_paused": paused }
            })),
        )
        .await
    }

    /// `POST /queues/{id}/messages` — publica un mensaje.
    pub async fn push_message(
        &self,
        account_id: &str,
        queue_id: &str,
        body: &str,
        content_type: &str,
        delay_seconds: Option<u64>,
    ) -> Result<()> {
        let mut payload = json!({ "body": body, "content_type": content_type });
        if let Some(delay) = delay_seconds {
            payload["delay_seconds"] = json!(delay);
        }
        self.send_ok(
            Method::POST,
            &format!("/accounts/{account_id}/queues/{queue_id}/messages"),
            Some(&payload),
        )
        .await
    }

    /// `POST /queues/{id}/purge` — borra TODOS los mensajes (irreversible).
    pub async fn purge_queue(&self, account_id: &str, queue_id: &str) -> Result<()> {
        self.send_ok(
            Method::POST,
            &format!("/accounts/{account_id}/queues/{queue_id}/purge"),
            Some(&json!({ "delete_messages_permanently": true })),
        )
        .await
    }

    /// `GET /queues/{id}/consumers` — consumers con settings completos.
    pub async fn list_consumers(
        &self,
        account_id: &str,
        queue_id: &str,
    ) -> Result<Vec<QueueConsumer>> {
        self.get(&format!(
            "/accounts/{account_id}/queues/{queue_id}/consumers"
        ))
        .await
    }

    /// `PUT /queues/{id}/consumers/{consumer_id}` — actualiza el consumer.
    /// El body (mismo shape que el GET) lo arma el formulario.
    pub async fn update_consumer(
        &self,
        account_id: &str,
        queue_id: &str,
        consumer_id: &str,
        body: &serde_json::Value,
    ) -> Result<()> {
        self.send_ok(
            Method::PUT,
            &format!("/accounts/{account_id}/queues/{queue_id}/consumers/{consumer_id}"),
            Some(body),
        )
        .await
    }

    /// `POST /queues/{id}/messages/pull` — peek: NUNCA se hace ack, así que
    /// los mensajes reaparecen tras `visibility_timeout_ms`.
    pub async fn pull_messages(
        &self,
        account_id: &str,
        queue_id: &str,
        batch_size: u64,
        visibility_timeout_ms: u64,
    ) -> Result<Vec<PulledMessage>> {
        // Envelope tolerante: `result.messages` o `result` como array.
        let v: serde_json::Value = self
            .post(
                &format!("/accounts/{account_id}/queues/{queue_id}/messages/pull"),
                &json!({
                    "batch_size": batch_size,
                    "visibility_timeout_ms": visibility_timeout_ms
                }),
            )
            .await?;
        let arr = v
            .get("messages")
            .cloned()
            .unwrap_or_else(|| if v.is_array() { v.clone() } else { json!([]) });
        Ok(serde_json::from_value(arr).unwrap_or_default())
    }

    /// Métricas de la cola en `[start, end]` (RFC3339) vía GraphQL: series de
    /// backlog y de mensajes ingeridos por hora. Backlog actual = último punto.
    pub async fn queue_metrics(
        &self,
        account_id: &str,
        queue_id: &str,
        start: &str,
        end: &str,
    ) -> Result<QueueMetrics> {
        let vars = json!({
            "accountTag": account_id,
            "queueId": queue_id,
            "start": start,
            "end": end,
        });
        let data: GqlData = self.graphql(QUEUE_METRICS_QUERY, vars).await?;
        let mut m = QueueMetrics::default();
        if let Some(account) = data.viewer.accounts.into_iter().next() {
            for g in account.backlog.unwrap_or_default() {
                m.series_backlog.push(g.avg.messages.round() as u64);
                // El backlog "actual" es el punto más reciente de la serie.
                m.backlog_messages = g.avg.messages.round() as u64;
                m.backlog_bytes = g.avg.bytes.round() as u64;
            }
            for g in account.ops.unwrap_or_default() {
                m.series_written.push(g.count);
            }
        }
        Ok(m)
    }
}
