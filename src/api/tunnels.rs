//! Endpoints de Túneles de Cloudflare (Zero Trust / cfd_tunnel), Fase 2.
//! Todos account-scoped: requieren el `account_id` de la cuenta activa.

use color_eyre::eyre::Result;
use serde::Deserialize;
use serde_json::json;

use super::CfClient;
use crate::model::{IngressRule, Tunnel};

/// Resultado de crear un túnel: incluye el `token` del conector.
#[derive(Debug, Deserialize)]
pub struct CreatedTunnel {
    #[allow(dead_code)]
    pub id: String,
    pub name: String,
    /// Token base64 para `cloudflared tunnel run --token <token>`.
    #[serde(default)]
    pub token: String,
}

/// Envelope de `.../configurations`: `{ config: { ingress: [...] } }`.
#[derive(Debug, Deserialize)]
struct TunnelConfig {
    #[serde(default)]
    config: Option<TunnelConfigInner>,
}

#[derive(Debug, Deserialize)]
struct TunnelConfigInner {
    #[serde(default)]
    ingress: Vec<IngressRule>,
}

impl CfClient {
    /// `GET /accounts/{id}/cfd_tunnel` — túneles no borrados.
    pub async fn list_tunnels(&self, account_id: &str) -> Result<Vec<Tunnel>> {
        self.get(&format!(
            "/accounts/{account_id}/cfd_tunnel?is_deleted=false&per_page=50"
        ))
        .await
    }

    /// `GET /accounts/{id}/cfd_tunnel/{tid}/configurations` — reglas de ingress.
    /// Solo túneles remotely-managed; los locales no tienen config remota.
    pub async fn tunnel_ingress(
        &self,
        account_id: &str,
        tunnel_id: &str,
    ) -> Result<Vec<IngressRule>> {
        let cfg: TunnelConfig = self
            .get(&format!(
                "/accounts/{account_id}/cfd_tunnel/{tunnel_id}/configurations"
            ))
            .await?;
        Ok(cfg.config.map(|c| c.ingress).unwrap_or_default())
    }

    /// `POST /accounts/{id}/cfd_tunnel` — crea un túnel remotely-managed.
    pub async fn create_tunnel(&self, account_id: &str, name: &str) -> Result<CreatedTunnel> {
        self.post(
            &format!("/accounts/{account_id}/cfd_tunnel"),
            &json!({ "name": name, "config_src": "cloudflare" }),
        )
        .await
    }

    /// `DELETE /accounts/{id}/cfd_tunnel/{tid}/connections` — limpia conexiones.
    pub async fn cleanup_tunnel_connections(
        &self,
        account_id: &str,
        tunnel_id: &str,
    ) -> Result<()> {
        self.delete_ok(&format!(
            "/accounts/{account_id}/cfd_tunnel/{tunnel_id}/connections"
        ))
        .await
    }

    /// `DELETE /accounts/{id}/cfd_tunnel/{tid}` — borra el túnel.
    /// Falla con 400 si tiene conexiones activas (limpiar primero).
    pub async fn delete_tunnel(&self, account_id: &str, tunnel_id: &str) -> Result<()> {
        self.delete_ok(&format!("/accounts/{account_id}/cfd_tunnel/{tunnel_id}"))
            .await
    }
}
