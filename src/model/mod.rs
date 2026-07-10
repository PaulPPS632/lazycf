//! Structs serde para las respuestas del API v4 de Cloudflare.
//! El envelope `CfResponse<T>` es común a casi todos los endpoints REST.

use serde::{Deserialize, Deserializer};

/// Algunos endpoints devuelven `null` (no solo el campo ausente) para listas
/// vacías o escalares; el `default` de serde por sí solo no cubre ese caso, así
/// que todos los campos con `#[serde(default)]` del modelo usan este helper.
fn null_as_default<'de, D, T>(de: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de> + Default,
{
    Ok(Option::deserialize(de)?.unwrap_or_default())
}

/// Envelope estándar del API v4: `{ success, errors, messages, result, result_info }`.
#[derive(Debug, Deserialize)]
pub struct CfResponse<T> {
    pub success: bool,
    #[serde(default, deserialize_with = "null_as_default")]
    pub errors: Vec<CfError>,
    #[serde(default, deserialize_with = "null_as_default")]
    #[allow(dead_code)] // se expondrá para avisos del API (Fase 1+)
    pub messages: Vec<serde_json::Value>,
    pub result: Option<T>,
    #[serde(default, deserialize_with = "null_as_default")]
    #[allow(dead_code)] // consumido al paginar listados (Fase 1+)
    pub result_info: Option<ResultInfo>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct CfError {
    #[serde(default, deserialize_with = "null_as_default")]
    #[allow(dead_code)] // disponible para logging/depuración
    pub code: i64,
    pub message: String,
}

/// Info de paginación (`page`, `total_count`, …). Se usará al listar recursos.
#[derive(Debug, Deserialize, Clone, Default)]
#[allow(dead_code)] // consumido al paginar listados (Fase 1+)
pub struct ResultInfo {
    #[serde(default, deserialize_with = "null_as_default")]
    pub page: u32,
    #[serde(default, deserialize_with = "null_as_default")]
    pub per_page: u32,
    #[serde(default, deserialize_with = "null_as_default")]
    pub total_count: u32,
    #[serde(default, deserialize_with = "null_as_default")]
    pub total_pages: u32,
}

/// Resultado de `GET /user/tokens/verify`.
#[derive(Debug, Deserialize, Clone)]
pub struct TokenVerify {
    #[allow(dead_code)] // id del token, útil para mostrar/depurar más adelante
    pub id: String,
    /// `active` | `disabled` | `expired`.
    pub status: String,
}

/// Cuenta de Cloudflare (`GET /accounts`).
#[derive(Debug, Deserialize, Clone)]
pub struct Account {
    pub id: String,
    pub name: String,
}

/// Zona (`GET /zones`).
#[derive(Debug, Deserialize, Clone)]
pub struct Zone {
    pub id: String,
    pub name: String,
    #[serde(default, deserialize_with = "null_as_default")]
    pub status: String,
    /// Cuenta dueña de la zona (para filtrar por cuenta activa).
    #[serde(default, deserialize_with = "null_as_default")]
    pub account: Option<ZoneAccount>,
}

#[derive(Debug, Deserialize, Clone, Default)]
pub struct ZoneAccount {
    pub id: String,
    #[serde(default, deserialize_with = "null_as_default")]
    #[allow(dead_code)] // nombre de la cuenta dueña; se puede mostrar más adelante
    pub name: String,
}

impl Zone {
    /// `id` de la cuenta dueña, si viene en la respuesta.
    pub fn account_id(&self) -> Option<&str> {
        self.account.as_ref().map(|a| a.id.as_str())
    }
}

/// Registro DNS (`GET /zones/{id}/dns_records`).
#[derive(Debug, Deserialize, Clone)]
pub struct DnsRecord {
    pub id: String,
    #[serde(rename = "type")]
    pub record_type: String,
    pub name: String,
    #[serde(default, deserialize_with = "null_as_default")]
    pub content: String,
    /// `Some(true/false)` en registros proxiables (A/AAAA/CNAME); `None` si no aplica.
    #[serde(default, deserialize_with = "null_as_default")]
    pub proxied: Option<bool>,
    /// TTL en segundos; `1` = automático.
    #[serde(default, deserialize_with = "null_as_default")]
    pub ttl: u32,
    /// Prioridad (MX/SRV); `None` para otros tipos.
    #[serde(default, deserialize_with = "null_as_default")]
    pub priority: Option<u32>,
}

impl DnsRecord {
    /// Solo A, AAAA y CNAME pueden pasar por el proxy (nube naranja).
    pub fn is_proxiable(&self) -> bool {
        matches!(self.record_type.as_str(), "A" | "AAAA" | "CNAME")
    }
}

/// Resultado mínimo `{ "id": ... }` (delete, purge).
#[derive(Debug, Deserialize, Clone)]
pub struct IdResult {
    #[allow(dead_code)] // confirma la operación; no siempre se muestra
    pub id: String,
}

/// Túnel de Cloudflare (`GET /accounts/{id}/cfd_tunnel`).
#[derive(Debug, Deserialize, Clone)]
pub struct Tunnel {
    pub id: String,
    pub name: String,
    /// `inactive` | `down` | `degraded` | `healthy`.
    #[serde(default, deserialize_with = "null_as_default")]
    pub status: String,
    /// Conexiones de conectores activas (vienen embebidas en el listado).
    #[serde(default, deserialize_with = "null_as_default")]
    pub connections: Vec<TunnelConnection>,
}

#[derive(Debug, Deserialize, Clone, Default)]
pub struct TunnelConnection {
    #[serde(default, deserialize_with = "null_as_default")]
    pub colo_name: String,
    #[serde(default, deserialize_with = "null_as_default")]
    pub origin_ip: String,
    #[serde(default, deserialize_with = "null_as_default")]
    pub client_version: String,
}

/// Script de Worker (`GET /accounts/{id}/workers/scripts`).
#[derive(Debug, Deserialize, Clone)]
pub struct WorkerScript {
    pub id: String,
    #[serde(default, deserialize_with = "null_as_default")]
    pub modified_on: String,
}

/// Métricas de un Worker (GraphQL `workersInvocationsAdaptive`, últimas 24h).
#[derive(Debug, Clone, Default)]
pub struct WorkerMetrics {
    pub requests: u64,
    pub errors: u64,
    pub cpu_p50: f64,
    pub cpu_p99: f64,
    /// Requests por hora (para el sparkline), en orden cronológico.
    pub series: Vec<u64>,
}

impl WorkerMetrics {
    /// Tasa de error en % (`errors / requests`).
    pub fn error_rate(&self) -> f64 {
        if self.requests == 0 {
            0.0
        } else {
            self.errors as f64 / self.requests as f64 * 100.0
        }
    }
}

/// Versión desplegada dentro de un deployment (para rollback multi-versión).
#[derive(Debug, Deserialize, Clone, Default)]
pub struct DeployVersion {
    #[serde(default, deserialize_with = "null_as_default")]
    pub version_id: String,
    /// Porcentaje de tráfico (soporta despliegues graduales, p. ej. 10.0).
    #[serde(default, deserialize_with = "null_as_default")]
    pub percentage: f64,
}

/// Implementación (deployment) de un Worker.
#[derive(Debug, Deserialize, Clone)]
pub struct Deployment {
    #[allow(dead_code)] // id del deployment (informativo)
    pub id: String,
    #[serde(default, deserialize_with = "null_as_default")]
    pub created_on: String,
    #[serde(default, deserialize_with = "null_as_default")]
    pub author_email: String,
    #[serde(default, deserialize_with = "null_as_default")]
    pub source: String,
    /// Versiones que sirve este deployment (para revertir preservando pesos).
    #[serde(default, deserialize_with = "null_as_default")]
    pub versions: Vec<DeployVersion>,
}

/// Ruta de zona que apunta a un Worker (`GET /zones/{id}/workers/routes`).
#[derive(Debug, Deserialize, Clone, Default)]
pub struct WorkerRoute {
    #[serde(default, deserialize_with = "null_as_default")]
    #[allow(dead_code)]
    pub id: String,
    #[serde(default, deserialize_with = "null_as_default")]
    pub pattern: String,
    #[serde(default, deserialize_with = "null_as_default")]
    pub script: Option<String>,
}

/// Custom domain de Workers (`GET /accounts/{id}/workers/domains`).
#[derive(Debug, Deserialize, Clone, Default)]
pub struct WorkerDomain {
    #[serde(default, deserialize_with = "null_as_default")]
    pub hostname: String,
    /// Nombre del script asociado.
    #[serde(default, deserialize_with = "null_as_default")]
    pub service: String,
}

/// Binding de un Worker (variable, secreto, KV, D1, R2, cola, AI…).
#[derive(Debug, Deserialize, Clone)]
pub struct Binding {
    pub name: String,
    #[serde(rename = "type")]
    pub btype: String,
    #[serde(default, deserialize_with = "null_as_default")]
    pub text: Option<String>,
    #[serde(default, deserialize_with = "null_as_default")]
    pub queue_name: Option<String>,
    #[serde(default, deserialize_with = "null_as_default")]
    pub namespace_id: Option<String>,
    #[serde(default, deserialize_with = "null_as_default")]
    pub bucket_name: Option<String>,
    #[serde(default, deserialize_with = "null_as_default")]
    pub database_id: Option<String>,
}

impl Binding {
    /// `true` si es un secreto (valor oculto).
    pub fn is_secret(&self) -> bool {
        self.btype == "secret_text"
    }

    /// Valor legible a mostrar (secretos enmascarados).
    pub fn display_value(&self) -> String {
        match self.btype.as_str() {
            "plain_text" => self.text.clone().unwrap_or_default(),
            "secret_text" => "••••••••".to_string(),
            "queue" => self.queue_name.clone().unwrap_or_default(),
            "kv_namespace" => self.namespace_id.clone().unwrap_or_default(),
            "r2_bucket" => self.bucket_name.clone().unwrap_or_default(),
            "d1" => self.database_id.clone().unwrap_or_default(),
            "ai" => "Workers AI".to_string(),
            _ => String::new(),
        }
    }
}

/// Base de datos D1 (`GET /accounts/{id}/d1/database`).
#[derive(Debug, Deserialize, Clone)]
pub struct D1Database {
    pub uuid: String,
    pub name: String,
    // Los contadores del listado son poco fiables (llegan a 0 con tablas reales);
    // se conservan para depuración pero no se muestran.
    #[serde(default, deserialize_with = "null_as_default")]
    #[allow(dead_code)]
    pub version: String,
    #[serde(default, deserialize_with = "null_as_default")]
    #[allow(dead_code)]
    pub num_tables: Option<u64>,
    #[serde(default, deserialize_with = "null_as_default")]
    #[allow(dead_code)]
    pub file_size: Option<u64>,
}

/// Resultado tabular de una consulta D1 (`POST .../raw`).
#[derive(Debug, Clone, Default)]
pub struct QueryOutcome {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<String>>,
    /// La consulta devolvió más filas que el tope de la rejilla.
    pub truncated: bool,
    pub rows_read: u64,
    pub rows_written: u64,
    pub changes: u64,
    pub duration_ms: f64,
}

impl QueryOutcome {
    /// Resumen de una línea con los contadores de `meta`.
    pub fn summary(&self) -> String {
        let trunc = if self.truncated {
            " · TRUNCADO (añade LIMIT/WHERE para refinar)"
        } else {
            ""
        };
        format!(
            "{} filas{trunc} · leídas {} · escritas {} · {} cambios · {:.2} ms",
            self.rows.len(),
            self.rows_read,
            self.rows_written,
            self.changes,
            self.duration_ms
        )
    }
}

/// Bucket R2 (`GET /accounts/{id}/r2/buckets`). El listado solo trae
/// `name`+`creation_date`; el detalle añade ubicación/clase/jurisdicción.
#[derive(Debug, Deserialize, Clone, Default)]
pub struct R2Bucket {
    pub name: String,
    #[serde(default, deserialize_with = "null_as_default")]
    pub creation_date: String,
    #[serde(default, deserialize_with = "null_as_default")]
    pub location: Option<String>,
    #[serde(default, deserialize_with = "null_as_default")]
    pub storage_class: Option<String>,
    #[serde(default, deserialize_with = "null_as_default")]
    #[allow(dead_code)] // no cabe en el panel compacto de info
    pub jurisdiction: Option<String>,
}

/// Uso de un bucket (`.../usage`). La API devuelve los tamaños como strings.
#[derive(Debug, Deserialize, Clone, Default)]
pub struct R2Usage {
    #[serde(rename = "payloadSize", default, deserialize_with = "null_as_default")]
    pub payload_size: String,
    #[serde(rename = "metadataSize", default, deserialize_with = "null_as_default")]
    pub metadata_size: String,
    #[serde(rename = "objectCount", default, deserialize_with = "null_as_default")]
    pub object_count: String,
}

/// Dominio personalizado de un bucket R2 (`.../domains/custom`).
#[derive(Debug, Deserialize, Clone)]
pub struct CustomDomain {
    pub domain: String,
    #[serde(default, deserialize_with = "null_as_default")]
    pub enabled: bool,
}

/// Dominio público administrado (`.../domains/managed`, r2.dev). El dominio
/// existe aunque `enabled` sea `false` (se pre-asigna al crear el bucket).
#[derive(Debug, Deserialize, Clone, Default)]
pub struct PublicDomain {
    #[serde(default, deserialize_with = "null_as_default")]
    pub domain: String,
    #[serde(default, deserialize_with = "null_as_default")]
    pub enabled: bool,
}

impl R2Usage {
    pub fn payload(&self) -> u64 {
        self.payload_size.parse().unwrap_or(0)
    }
    pub fn metadata(&self) -> u64 {
        self.metadata_size.parse().unwrap_or(0)
    }
    pub fn objects(&self) -> u64 {
        self.object_count.parse().unwrap_or(0)
    }
}

/// Objeto R2 (`GET .../r2/buckets/{b}/objects`).
#[derive(Debug, Deserialize, Clone)]
pub struct R2Object {
    pub key: String,
    #[serde(default, deserialize_with = "null_as_default")]
    pub size: u64,
    #[serde(default, deserialize_with = "null_as_default")]
    pub last_modified: String,
    #[serde(default, deserialize_with = "null_as_default")]
    pub http_metadata: Option<R2HttpMeta>,
}

#[derive(Debug, Deserialize, Clone, Default)]
pub struct R2HttpMeta {
    #[serde(rename = "contentType", default, deserialize_with = "null_as_default")]
    pub content_type: Option<String>,
}

impl R2Object {
    /// Nombre del archivo (última parte de la clave).
    pub fn filename(&self) -> &str {
        self.key.rsplit('/').next().unwrap_or(&self.key)
    }

    /// `true` si parece una imagen (por content-type o extensión).
    pub fn is_image(&self) -> bool {
        if let Some(ct) = self
            .http_metadata
            .as_ref()
            .and_then(|m| m.content_type.as_deref())
            && ct.starts_with("image/")
        {
            return true;
        }
        let lower = self.key.to_lowercase();
        [".png", ".jpg", ".jpeg", ".gif", ".webp"]
            .iter()
            .any(|ext| lower.ends_with(ext))
    }
}

/// Regla de ingress (`.../configurations`): hostname público → servicio local.
#[derive(Debug, Deserialize, Clone, Default)]
pub struct IngressRule {
    #[serde(default, deserialize_with = "null_as_default")]
    pub hostname: String,
    #[serde(default, deserialize_with = "null_as_default")]
    pub service: String,
    #[serde(default, deserialize_with = "null_as_default")]
    pub path: Option<String>,
}

/// Cola (`GET /accounts/{id}/queues`).
#[derive(Debug, Deserialize, Clone)]
pub struct Queue {
    pub queue_id: String,
    pub queue_name: String,
    #[serde(default, deserialize_with = "null_as_default")]
    pub created_on: String,
    #[serde(default, deserialize_with = "null_as_default")]
    pub modified_on: String,
    #[serde(default, deserialize_with = "null_as_default")]
    pub producers_total_count: u64,
    #[serde(default, deserialize_with = "null_as_default")]
    pub consumers_total_count: u64,
    /// Producers embebidos en el listado (solo para el Resumen). El API
    /// devuelve `null` (no solo el campo ausente) cuando no hay ninguno.
    #[serde(default, deserialize_with = "null_as_default")]
    pub producers: Vec<QueueProducer>,
    /// Consumers embebidos: para el Resumen y el gating de peek/logs; la
    /// pestaña Consumers usa `GET .../consumers` (settings completos).
    #[serde(default, deserialize_with = "null_as_default")]
    pub consumers: Vec<QueueConsumer>,
    #[serde(default, deserialize_with = "null_as_default")]
    pub settings: QueueSettings,
}

#[derive(Debug, Deserialize, Clone, Default)]
pub struct QueueSettings {
    #[serde(default, deserialize_with = "null_as_default")]
    pub delivery_delay: Option<u64>,
    #[serde(default, deserialize_with = "null_as_default")]
    pub delivery_paused: bool,
    #[serde(default, deserialize_with = "null_as_default")]
    pub message_retention_period: Option<u64>,
}

/// Producer embebido de una cola (`type` = "worker" | "r2_bucket").
#[derive(Debug, Deserialize, Clone, Default)]
pub struct QueueProducer {
    #[serde(rename = "type", default, deserialize_with = "null_as_default")]
    pub ptype: String,
    /// El API usa `script` en unos endpoints y `script_name` en otros.
    #[serde(default, alias = "script")]
    pub script_name: Option<String>,
    #[serde(default, deserialize_with = "null_as_default")]
    pub bucket_name: Option<String>,
}

/// Consumer de una cola (`type` = "worker" | "http_pull").
#[derive(Debug, Deserialize, Clone, Default)]
pub struct QueueConsumer {
    /// Puede faltar en consumers antiguos → cadena vacía.
    #[serde(default, deserialize_with = "null_as_default")]
    pub consumer_id: String,
    #[serde(rename = "type", default, deserialize_with = "null_as_default")]
    pub ctype: String,
    #[serde(default, alias = "script")]
    pub script_name: Option<String>,
    #[serde(default, deserialize_with = "null_as_default")]
    pub dead_letter_queue: Option<String>,
    #[serde(default, deserialize_with = "null_as_default")]
    pub settings: ConsumerSettings,
}

impl QueueConsumer {
    pub fn is_worker(&self) -> bool {
        self.ctype == "worker"
    }

    /// Etiqueta legible: nombre del script o "HTTP pull".
    pub fn label(&self) -> String {
        match &self.script_name {
            Some(s) if !s.is_empty() => s.clone(),
            _ => "HTTP pull".to_string(),
        }
    }
}

#[derive(Debug, Deserialize, Clone, Default)]
pub struct ConsumerSettings {
    #[serde(default, deserialize_with = "null_as_default")]
    pub batch_size: Option<u64>,
    #[serde(default, deserialize_with = "null_as_default")]
    pub max_retries: Option<u64>,
    #[serde(default, deserialize_with = "null_as_default")]
    pub retry_delay: Option<u64>,
    /// Solo consumers worker.
    #[serde(default, deserialize_with = "null_as_default")]
    pub max_concurrency: Option<u64>,
    /// Solo consumers worker.
    #[serde(default, deserialize_with = "null_as_default")]
    pub max_wait_time_ms: Option<u64>,
    /// Solo consumers http_pull.
    #[serde(default, deserialize_with = "null_as_default")]
    pub visibility_timeout_ms: Option<u64>,
}

/// Mensaje espiado vía `POST .../messages/pull` (peek: nunca se hace ack).
#[derive(Debug, Deserialize, Clone, Default)]
pub struct PulledMessage {
    #[serde(default, deserialize_with = "null_as_default")]
    pub id: String,
    #[serde(default, deserialize_with = "null_as_default")]
    pub body: String,
    #[serde(default, deserialize_with = "null_as_default")]
    pub timestamp_ms: Option<i64>,
    #[serde(default, deserialize_with = "null_as_default")]
    pub attempts: u64,
}

/// Métricas de una cola (GraphQL, últimas 24h). Espejo de `WorkerMetrics`.
#[derive(Debug, Clone, Default)]
pub struct QueueMetrics {
    /// Backlog actual (último punto de la serie).
    pub backlog_messages: u64,
    pub backlog_bytes: u64,
    /// Backlog medio por hora, orden cronológico (sparkline).
    pub series_backlog: Vec<u64>,
    /// Mensajes ingeridos (WriteMessage) por hora (sparkline).
    pub series_written: Vec<u64>,
}

#[cfg(test)]
mod tests {
    use super::{CfResponse, DnsRecord, Queue, R2Bucket, Tunnel, Zone};

    /// El endurecimiento `null_as_default` es uniforme: cualquier campo escalar
    /// o de lista con `#[serde(default)]` tolera `null` explícito, no solo la
    /// familia Queues. Cubre DNS, zonas, túneles y buckets.
    #[test]
    fn null_explicito_tolerado_en_todos_los_modelos() {
        let zone: Zone =
            serde_json::from_str(r#"{"id":"z","name":"ej.com","status":null,"account":null}"#)
                .expect("Zone tolera status/account null");
        assert_eq!(zone.status, "");
        assert!(zone.account_id().is_none());

        let rec: DnsRecord = serde_json::from_str(
            r#"{"id":"r","type":"A","name":"a.ej.com","content":null,"proxied":null,"ttl":null,"priority":null}"#,
        )
        .expect("DnsRecord tolera content/ttl null");
        assert_eq!(rec.content, "");
        assert_eq!(rec.ttl, 0);

        let tunnel: Tunnel =
            serde_json::from_str(r#"{"id":"t","name":"tun","status":null,"connections":null}"#)
                .expect("Tunnel tolera status/connections null");
        assert_eq!(tunnel.status, "");
        assert!(tunnel.connections.is_empty());

        let bucket: R2Bucket = serde_json::from_str(r#"{"name":"b","creation_date":null}"#)
            .expect("R2Bucket tolera creation_date null");
        assert_eq!(bucket.name, "b");
    }

    /// El API real de Queues manda `null` explícito (no el campo ausente) en
    /// `producers`/`consumers`/contadores/fechas/booleanos cuando están vacíos
    /// o vienen de una cola vieja. Reproduce el bug reportado: una cola sin
    /// producers/consumers en medio de un array más largo.
    #[test]
    fn queue_con_nulls_no_rompe_el_parseo() {
        let body = r#"{
            "success": true,
            "errors": null,
            "messages": null,
            "result": [
                {
                    "queue_id": "aaa",
                    "queue_name": "con-datos",
                    "created_on": "2026-06-17T16:22:57.330165Z",
                    "modified_on": "2026-06-17T16:22:57.330165Z",
                    "producers_total_count": 1,
                    "consumers_total_count": 1,
                    "producers": [{"type": "worker", "script": "prod-script"}],
                    "consumers": [{"type": "worker", "script": "cons-script"}],
                    "settings": {"delivery_delay": 0, "delivery_paused": false, "message_retention_period": 345600}
                },
                {
                    "queue_id": "bbb",
                    "queue_name": "sin-datos",
                    "created_on": null,
                    "modified_on": null,
                    "producers_total_count": null,
                    "consumers_total_count": null,
                    "producers": null,
                    "consumers": null,
                    "settings": {"delivery_delay": null, "delivery_paused": null, "message_retention_period": null}
                }
            ],
            "result_info": null
        }"#;
        let parsed: CfResponse<Vec<Queue>> =
            serde_json::from_str(body).expect("debe tolerar nulls explícitos");
        let queues = parsed.result.expect("result presente");
        assert_eq!(queues.len(), 2);
        assert_eq!(queues[0].producers.len(), 1);
        let vacia = &queues[1];
        assert!(vacia.producers.is_empty());
        assert!(vacia.consumers.is_empty());
        assert_eq!(vacia.producers_total_count, 0);
        assert!(!vacia.settings.delivery_paused);
    }
}
