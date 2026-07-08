//! Structs serde para las respuestas del API v4 de Cloudflare.
//! El envelope `CfResponse<T>` es común a casi todos los endpoints REST.

use serde::Deserialize;

/// Envelope estándar del API v4: `{ success, errors, messages, result, result_info }`.
#[derive(Debug, Deserialize)]
pub struct CfResponse<T> {
    pub success: bool,
    #[serde(default)]
    pub errors: Vec<CfError>,
    #[serde(default)]
    #[allow(dead_code)] // se expondrá para avisos del API (Fase 1+)
    pub messages: Vec<serde_json::Value>,
    pub result: Option<T>,
    #[serde(default)]
    #[allow(dead_code)] // consumido al paginar listados (Fase 1+)
    pub result_info: Option<ResultInfo>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct CfError {
    #[serde(default)]
    #[allow(dead_code)] // disponible para logging/depuración
    pub code: i64,
    pub message: String,
}

/// Info de paginación (`page`, `total_count`, …). Se usará al listar recursos.
#[derive(Debug, Deserialize, Clone, Default)]
#[allow(dead_code)] // consumido al paginar listados (Fase 1+)
pub struct ResultInfo {
    #[serde(default)]
    pub page: u32,
    #[serde(default)]
    pub per_page: u32,
    #[serde(default)]
    pub total_count: u32,
    #[serde(default)]
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
    #[serde(default)]
    pub status: String,
    /// Cuenta dueña de la zona (para filtrar por cuenta activa).
    #[serde(default)]
    pub account: Option<ZoneAccount>,
}

#[derive(Debug, Deserialize, Clone, Default)]
pub struct ZoneAccount {
    pub id: String,
    #[serde(default)]
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
    #[serde(default)]
    pub content: String,
    /// `Some(true/false)` en registros proxiables (A/AAAA/CNAME); `None` si no aplica.
    #[serde(default)]
    pub proxied: Option<bool>,
    /// TTL en segundos; `1` = automático.
    #[serde(default)]
    pub ttl: u32,
    /// Prioridad (MX/SRV); `None` para otros tipos.
    #[serde(default)]
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
    #[serde(default)]
    pub status: String,
    /// Conexiones de conectores activas (vienen embebidas en el listado).
    #[serde(default)]
    pub connections: Vec<TunnelConnection>,
}

#[derive(Debug, Deserialize, Clone, Default)]
pub struct TunnelConnection {
    #[serde(default)]
    pub colo_name: String,
    #[serde(default)]
    pub origin_ip: String,
    #[serde(default)]
    pub client_version: String,
}

/// Script de Worker (`GET /accounts/{id}/workers/scripts`).
#[derive(Debug, Deserialize, Clone)]
pub struct WorkerScript {
    pub id: String,
    #[serde(default)]
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

/// Implementación (deployment) de un Worker.
#[derive(Debug, Deserialize, Clone)]
pub struct Deployment {
    #[allow(dead_code)] // id del deployment; para rollback en el futuro
    pub id: String,
    #[serde(default)]
    pub created_on: String,
    #[serde(default)]
    pub author_email: String,
    #[serde(default)]
    pub source: String,
}

/// Binding de un Worker (variable, secreto, KV, D1, R2, cola, AI…).
#[derive(Debug, Deserialize, Clone)]
pub struct Binding {
    pub name: String,
    #[serde(rename = "type")]
    pub btype: String,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub queue_name: Option<String>,
    #[serde(default)]
    pub namespace_id: Option<String>,
    #[serde(default)]
    pub bucket_name: Option<String>,
    #[serde(default)]
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
    #[serde(default)]
    #[allow(dead_code)]
    pub version: String,
    #[serde(default)]
    #[allow(dead_code)]
    pub num_tables: Option<u64>,
    #[serde(default)]
    #[allow(dead_code)]
    pub file_size: Option<u64>,
}

/// Resultado tabular de una consulta D1 (`POST .../raw`).
#[derive(Debug, Clone, Default)]
pub struct QueryOutcome {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<String>>,
    pub rows_read: u64,
    pub rows_written: u64,
    pub changes: u64,
    pub duration_ms: f64,
}

impl QueryOutcome {
    /// Resumen de una línea con los contadores de `meta`.
    pub fn summary(&self) -> String {
        format!(
            "{} filas · leídas {} · escritas {} · {} cambios · {:.2} ms",
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
    #[serde(default)]
    pub creation_date: String,
    #[serde(default)]
    pub location: Option<String>,
    #[serde(default)]
    pub storage_class: Option<String>,
    #[serde(default)]
    #[allow(dead_code)] // no cabe en el panel compacto de info
    pub jurisdiction: Option<String>,
}

/// Uso de un bucket (`.../usage`). La API devuelve los tamaños como strings.
#[derive(Debug, Deserialize, Clone, Default)]
pub struct R2Usage {
    #[serde(rename = "payloadSize", default)]
    pub payload_size: String,
    #[serde(rename = "metadataSize", default)]
    pub metadata_size: String,
    #[serde(rename = "objectCount", default)]
    pub object_count: String,
}

/// Dominio personalizado de un bucket R2 (`.../domains/custom`).
#[derive(Debug, Deserialize, Clone)]
pub struct CustomDomain {
    pub domain: String,
    #[serde(default)]
    pub enabled: bool,
}

/// Dominio público administrado (`.../domains/managed`, r2.dev). El dominio
/// existe aunque `enabled` sea `false` (se pre-asigna al crear el bucket).
#[derive(Debug, Deserialize, Clone, Default)]
pub struct PublicDomain {
    #[serde(default)]
    pub domain: String,
    #[serde(default)]
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
    #[serde(default)]
    pub size: u64,
    #[serde(default)]
    pub last_modified: String,
    #[serde(default)]
    pub http_metadata: Option<R2HttpMeta>,
}

#[derive(Debug, Deserialize, Clone, Default)]
pub struct R2HttpMeta {
    #[serde(rename = "contentType", default)]
    pub content_type: Option<String>,
}

impl R2Object {
    /// Nombre del archivo (última parte de la clave).
    pub fn filename(&self) -> &str {
        self.key.rsplit('/').next().unwrap_or(&self.key)
    }

    /// `true` si parece una imagen (por content-type o extensión).
    pub fn is_image(&self) -> bool {
        if let Some(ct) = self.http_metadata.as_ref().and_then(|m| m.content_type.as_deref())
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
    #[serde(default)]
    pub hostname: String,
    #[serde(default)]
    pub service: String,
    #[serde(default)]
    pub path: Option<String>,
}
