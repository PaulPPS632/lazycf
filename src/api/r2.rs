//! Endpoints de R2 (Fase 6): buckets y objetos vía Bearer (v4 REST, como
//! wrangler) + URLs prefirmadas S3 (SigV4 manual, requiere credenciales R2).

use color_eyre::eyre::Result;
use percent_encoding::{percent_encode, AsciiSet, NON_ALPHANUMERIC};
use serde::Deserialize;
use serde_json::json;

use super::CfClient;
use crate::model::{CustomDomain, PublicDomain, R2Bucket, R2Object, R2Usage};

/// Codificación estilo AWS: todo salvo no-reservados (`A-Za-z0-9 - . _ ~`).
const AWS_ENC: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~');

/// Como `AWS_ENC` pero conservando `/` (para el path canónico).
const AWS_PATH_ENC: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~')
    .remove(b'/');

/// Clave de objeto codificada para la ruta REST (v4 exige `/` codificado).
fn key_enc(key: &str) -> String {
    percent_encode(key.as_bytes(), AWS_ENC).to_string()
}

/// URL pública de un objeto servido por un dominio (público r2.dev o
/// personalizado). Conserva `/` en la clave (carpetas virtuales).
pub fn object_url(domain: &str, key: &str) -> String {
    format!("https://{domain}/{}", percent_encode(key.as_bytes(), AWS_PATH_ENC))
}

/// Resultado de listar objetos con `delimiter=/`.
#[derive(Debug, Clone, Default)]
pub struct ObjectList {
    pub folders: Vec<String>,
    pub files: Vec<R2Object>,
    pub truncated: bool,
}

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
    pub async fn bucket_domains(&self, account_id: &str, name: &str) -> Result<Vec<CustomDomain>> {
        #[derive(Deserialize)]
        struct Resp {
            #[serde(default)]
            domains: Vec<CustomDomain>,
        }
        let r: Resp = self
            .get(&format!(
                "/accounts/{account_id}/r2/buckets/{name}/domains/custom"
            ))
            .await?;
        Ok(r.domains)
    }

    /// `GET .../buckets/{name}/domains/managed` — dominio público r2.dev. El
    /// dominio existe aunque `enabled` sea `false` (pre-asignado al crear el bucket).
    pub async fn bucket_public_domain(&self, account_id: &str, name: &str) -> Result<PublicDomain> {
        self.get(&format!(
            "/accounts/{account_id}/r2/buckets/{name}/domains/managed"
        ))
        .await
    }

    /// `GET .../buckets/{name}/cors` — reglas CORS crudas (vacío si no hay política).
    pub async fn bucket_cors(&self, account_id: &str, name: &str) -> Result<Vec<serde_json::Value>> {
        #[derive(Deserialize, Default)]
        struct Resp {
            #[serde(default)]
            rules: Vec<serde_json::Value>,
        }
        let v = self
            .get_value(&format!("/accounts/{account_id}/r2/buckets/{name}/cors"))
            .await?;
        let r: Resp = serde_json::from_value(v["result"].clone()).unwrap_or_default();
        Ok(r.rules)
    }

    /// `PUT .../buckets/{name}/cors` — reemplaza la política CORS completa.
    pub async fn set_bucket_cors(
        &self,
        account_id: &str,
        name: &str,
        rules: serde_json::Value,
    ) -> Result<()> {
        self.send_ok(
            reqwest::Method::PUT,
            &format!("/accounts/{account_id}/r2/buckets/{name}/cors"),
            Some(&json!({ "rules": rules })),
        )
        .await
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

    /// `GET .../objects?delimiter=/&prefix=…` — carpetas (`delimited`) + archivos.
    pub async fn list_objects(
        &self,
        account_id: &str,
        bucket: &str,
        prefix: &str,
    ) -> Result<ObjectList> {
        let path = format!(
            "/accounts/{account_id}/r2/buckets/{bucket}/objects?per_page=500&delimiter=%2F&prefix={}",
            percent_encode(prefix.as_bytes(), AWS_ENC)
        );
        let v = self.get_value(&path).await?;
        let files: Vec<R2Object> =
            serde_json::from_value(v["result"].clone()).unwrap_or_default();
        let folders: Vec<String> = v["result_info"]["delimited"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|d| d.as_str())
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();
        let truncated = v["result_info"]["is_truncated"].as_bool().unwrap_or(false);
        Ok(ObjectList {
            folders,
            files,
            truncated,
        })
    }

    /// `GET .../objects/{key}` — descarga el cuerpo del objeto.
    pub async fn get_object(&self, account_id: &str, bucket: &str, key: &str) -> Result<Vec<u8>> {
        self.get_bytes(&format!(
            "/accounts/{account_id}/r2/buckets/{bucket}/objects/{}",
            key_enc(key)
        ))
        .await
    }

    /// `PUT .../objects/{key}` — sube un objeto con su content-type.
    pub async fn put_object(
        &self,
        account_id: &str,
        bucket: &str,
        key: &str,
        body: Vec<u8>,
        content_type: &str,
    ) -> Result<()> {
        self.put_bytes(
            &format!(
                "/accounts/{account_id}/r2/buckets/{bucket}/objects/{}",
                key_enc(key)
            ),
            body,
            content_type,
        )
        .await
    }

    /// `DELETE .../objects/{key}` — borra un objeto.
    pub async fn delete_object(&self, account_id: &str, bucket: &str, key: &str) -> Result<()> {
        self.delete_ok(&format!(
            "/accounts/{account_id}/r2/buckets/{bucket}/objects/{}",
            key_enc(key)
        ))
        .await
    }
}

/// URL prefirmada (GET) contra el endpoint S3 de R2, firma SigV4 en query
/// (`X-Amz-*`). Cálculo local: no toca la red. `expires` en segundos (1s–7d).
pub fn presign_get(
    account_id: &str,
    access_key: &str,
    secret: &str,
    bucket: &str,
    key: &str,
    expires: u64,
    now: chrono::DateTime<chrono::Utc>,
) -> String {
    use hmac::{Hmac, Mac};
    use sha2::{Digest, Sha256};
    type HmacSha256 = Hmac<Sha256>;

    fn hmac(key: &[u8], data: &str) -> Vec<u8> {
        let mut mac = HmacSha256::new_from_slice(key).expect("hmac acepta cualquier tamaño");
        mac.update(data.as_bytes());
        mac.finalize().into_bytes().to_vec()
    }

    let host = format!("{account_id}.r2.cloudflarestorage.com");
    let amz_date = now.format("%Y%m%dT%H%M%SZ").to_string();
    let datestamp = now.format("%Y%m%d").to_string();
    let scope = format!("{datestamp}/auto/s3/aws4_request");
    let raw_credential = format!("{access_key}/{scope}");
    let credential = percent_encode(raw_credential.as_bytes(), AWS_ENC);

    let canonical_uri = format!(
        "/{bucket}/{}",
        percent_encode(key.as_bytes(), AWS_PATH_ENC)
    );
    // Parámetros en orden alfabético (requisito de la firma).
    let query = format!(
        "X-Amz-Algorithm=AWS4-HMAC-SHA256&X-Amz-Credential={credential}\
         &X-Amz-Date={amz_date}&X-Amz-Expires={expires}&X-Amz-SignedHeaders=host"
    );
    let canonical_request =
        format!("GET\n{canonical_uri}\n{query}\nhost:{host}\n\nhost\nUNSIGNED-PAYLOAD");
    let hashed = hex::encode(Sha256::digest(canonical_request.as_bytes()));
    let string_to_sign = format!("AWS4-HMAC-SHA256\n{amz_date}\n{scope}\n{hashed}");

    let k_date = hmac(format!("AWS4{secret}").as_bytes(), &datestamp);
    let k_region = hmac(&k_date, "auto");
    let k_service = hmac(&k_region, "s3");
    let k_signing = hmac(&k_service, "aws4_request");
    let signature = hex::encode(hmac(&k_signing, &string_to_sign));

    format!("https://{host}{canonical_uri}?{query}&X-Amz-Signature={signature}")
}
