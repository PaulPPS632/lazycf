//! Endpoints de R2 (Fase 6): gestión de buckets vía Bearer (v4 REST).
//! Los objetos (browser/subida/URLs firmadas) van por la API S3 con
//! credenciales R2 aparte → incremento posterior (aws-sdk-s3).

use color_eyre::eyre::Result;
use serde::Deserialize;
use serde_json::json;

use super::CfClient;
use crate::model::{R2Bucket, R2Usage};

impl CfClient {
    /// `GET /accounts/{id}/r2/buckets` — buckets de la cuenta.
    pub async fn list_buckets(&self, account_id: &str) -> Result<Vec<R2Bucket>> {
        #[derive(Deserialize)]
        struct Resp {
            #[serde(default)]
            buckets: Vec<R2Bucket>,
        }
        let r: Resp = self
            .get(&format!("/accounts/{account_id}/r2/buckets"))
            .await?;
        Ok(r.buckets)
    }

    /// `GET /accounts/{id}/r2/buckets/{name}` — detalle (ubicación/clase/jurisdicción).
    pub async fn bucket_detail(&self, account_id: &str, name: &str) -> Result<R2Bucket> {
        self.get(&format!("/accounts/{account_id}/r2/buckets/{name}"))
            .await
    }

    /// `GET /accounts/{id}/r2/buckets/{name}/usage` — objetos y tamaño almacenado.
    pub async fn bucket_usage(&self, account_id: &str, name: &str) -> Result<R2Usage> {
        self.get(&format!("/accounts/{account_id}/r2/buckets/{name}/usage"))
            .await
    }

    /// `GET .../buckets/{name}/domains/custom` — dominios personalizados del bucket.
    pub async fn bucket_domains(&self, account_id: &str, name: &str) -> Result<Vec<String>> {
        #[derive(Deserialize)]
        struct Resp {
            #[serde(default)]
            domains: Vec<Domain>,
        }
        #[derive(Deserialize)]
        struct Domain {
            #[serde(default)]
            domain: String,
            #[serde(default)]
            enabled: bool,
        }
        let r: Resp = self
            .get(&format!(
                "/accounts/{account_id}/r2/buckets/{name}/domains/custom"
            ))
            .await?;
        Ok(r.domains
            .into_iter()
            .map(|d| {
                if d.enabled {
                    d.domain
                } else {
                    format!("{} (deshabilitado)", d.domain)
                }
            })
            .collect())
    }

    /// `POST /accounts/{id}/r2/buckets` — crea un bucket.
    pub async fn create_bucket(&self, account_id: &str, name: &str) -> Result<()> {
        let body = json!({ "name": name });
        self.send_ok(
            reqwest::Method::POST,
            &format!("/accounts/{account_id}/r2/buckets"),
            Some(&body),
        )
        .await
    }

    /// `DELETE /accounts/{id}/r2/buckets/{name}` — borra un bucket (debe estar vacío).
    pub async fn delete_bucket(&self, account_id: &str, name: &str) -> Result<()> {
        self.delete_ok(&format!("/accounts/{account_id}/r2/buckets/{name}"))
            .await
    }
}
