//! Endpoints de Túneles de Cloudflare (Zero Trust / cfd_tunnel), Fase 2.
//! Todos account-scoped: requieren el `account_id` de la cuenta activa.

use color_eyre::eyre::{Result, bail};
use reqwest::Method;
use serde::Deserialize;
use serde_json::{Value, json};

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

    /// Config actual del túnel (envelope → `result.config`) junto con sus reglas
    /// de ingress SIN la catch-all final (para poder mutarlas y re-añadirla).
    async fn tunnel_config_ingress(
        &self,
        account_id: &str,
        tunnel_id: &str,
    ) -> Result<(Value, Vec<Value>)> {
        let cfg_path = format!("/accounts/{account_id}/cfd_tunnel/{tunnel_id}/configurations");
        let current = self.get_value(&cfg_path).await?;
        let mut config = current["result"]["config"].clone();
        if !config.is_object() {
            config = json!({});
        }
        let ingress: Vec<Value> = config["ingress"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter(|r| {
                        r.get("hostname")
                            .and_then(Value::as_str)
                            .is_some_and(|h| !h.is_empty())
                    })
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();
        Ok((config, ingress))
    }

    /// Re-añade la catch-all obligatoria y hace `PUT` de la config completa
    /// (preserva originRequest, warp-routing, etc.).
    async fn put_tunnel_ingress(
        &self,
        account_id: &str,
        tunnel_id: &str,
        mut config: Value,
        mut ingress: Vec<Value>,
    ) -> Result<()> {
        ingress.push(json!({ "service": "http_status:404" }));
        config["ingress"] = Value::Array(ingress);
        let cfg_path = format!("/accounts/{account_id}/cfd_tunnel/{tunnel_id}/configurations");
        self.send_ok(Method::PUT, &cfg_path, Some(&json!({ "config": config })))
            .await
    }

    /// Añade una ruta pública (regla de ingress: hostname → servicio local) a un
    /// túnel remotely-managed. NO crea el DNS: el CNAME
    /// `hostname → {tunnel_id}.cfargotunnel.com` es una llamada aparte.
    pub async fn add_tunnel_route(
        &self,
        account_id: &str,
        tunnel_id: &str,
        hostname: &str,
        service: &str,
        path: Option<&str>,
    ) -> Result<()> {
        let (config, mut ingress) = self.tunnel_config_ingress(account_id, tunnel_id).await?;
        let mut rule = json!({ "hostname": hostname, "service": service });
        if let Some(p) = path.filter(|p| !p.is_empty()) {
            rule["path"] = json!(p);
        }
        ingress.push(rule);
        self.put_tunnel_ingress(account_id, tunnel_id, config, ingress)
            .await
    }

    /// Edita la regla de ingress cuyo hostname coincide: fija servicio y ruta.
    /// El hostname no cambia (así el CNAME sigue siendo válido).
    pub async fn update_tunnel_route(
        &self,
        account_id: &str,
        tunnel_id: &str,
        hostname: &str,
        service: &str,
        path: Option<&str>,
    ) -> Result<()> {
        let (config, mut ingress) = self.tunnel_config_ingress(account_id, tunnel_id).await?;
        let rule = ingress
            .iter_mut()
            .find(|r| r.get("hostname").and_then(Value::as_str) == Some(hostname));
        let Some(rule) = rule else {
            bail!("No se encontró la ruta {hostname}");
        };
        rule["service"] = json!(service);
        match path.filter(|p| !p.is_empty()) {
            Some(p) => rule["path"] = json!(p),
            None => {
                if let Some(obj) = rule.as_object_mut() {
                    obj.remove("path");
                }
            }
        }
        self.put_tunnel_ingress(account_id, tunnel_id, config, ingress)
            .await
    }

    /// Borra la regla de ingress cuyo hostname coincide. NO borra el CNAME.
    pub async fn delete_tunnel_route(
        &self,
        account_id: &str,
        tunnel_id: &str,
        hostname: &str,
    ) -> Result<()> {
        let (config, mut ingress) = self.tunnel_config_ingress(account_id, tunnel_id).await?;
        let before = ingress.len();
        ingress.retain(|r| r.get("hostname").and_then(Value::as_str) != Some(hostname));
        if ingress.len() == before {
            bail!("No se encontró la ruta {hostname}");
        }
        self.put_tunnel_ingress(account_id, tunnel_id, config, ingress)
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
