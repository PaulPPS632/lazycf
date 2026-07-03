//! Endpoints de DNS y caché (Fase 1). Vía el caller genérico `CfClient`.

use color_eyre::eyre::Result;
use serde_json::{json, Value};

use super::CfClient;
use crate::model::{DnsRecord, IdResult, Zone};

impl CfClient {
    /// `GET /zones` — zonas visibles para el token (una página, hasta 50).
    pub async fn list_zones(&self) -> Result<Vec<Zone>> {
        self.get("/zones?per_page=50").await
    }

    /// `GET /zones/{id}/dns_records` — registros de la zona (hasta 100).
    pub async fn list_dns_records(&self, zone_id: &str) -> Result<Vec<DnsRecord>> {
        self.get(&format!("/zones/{zone_id}/dns_records?per_page=100"))
            .await
    }

    /// `PATCH /zones/{id}/dns_records/{rid}` — cambia solo el flag `proxied`.
    pub async fn set_dns_proxied(
        &self,
        zone_id: &str,
        record_id: &str,
        proxied: bool,
    ) -> Result<DnsRecord> {
        self.patch(
            &format!("/zones/{zone_id}/dns_records/{record_id}"),
            &json!({ "proxied": proxied }),
        )
        .await
    }

    /// `POST /zones/{id}/dns_records` — crea un registro.
    pub async fn create_dns_record(&self, zone_id: &str, body: &Value) -> Result<DnsRecord> {
        self.post(&format!("/zones/{zone_id}/dns_records"), body).await
    }

    /// `PUT /zones/{id}/dns_records/{rid}` — reemplaza un registro.
    pub async fn update_dns_record(
        &self,
        zone_id: &str,
        record_id: &str,
        body: &Value,
    ) -> Result<DnsRecord> {
        self.put(&format!("/zones/{zone_id}/dns_records/{record_id}"), body)
            .await
    }

    /// `DELETE /zones/{id}/dns_records/{rid}`.
    pub async fn delete_dns_record(&self, zone_id: &str, record_id: &str) -> Result<IdResult> {
        self.delete(&format!("/zones/{zone_id}/dns_records/{record_id}"))
            .await
    }

    /// `POST /zones/{id}/purge_cache` con `{ purge_everything: true }`.
    pub async fn purge_everything(&self, zone_id: &str) -> Result<IdResult> {
        self.post(
            &format!("/zones/{zone_id}/purge_cache"),
            &json!({ "purge_everything": true }),
        )
        .await
    }
}
