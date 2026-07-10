//! Facade del API de Cloudflare. `client.rs` da el caller genérico; aquí se
//! cuelgan los helpers por dominio (auth ahora; DNS, tunnels, … en fases).

pub mod client;
pub mod d1;
pub mod dns;
pub mod queues;
pub mod r2;
pub mod tunnels;
pub mod workers;

pub use client::{CfClient, CredentialSource};

use color_eyre::eyre::{Result, bail};

use crate::model::{Account, TokenVerify};

/// Resultado de autenticación: cuentas visibles para el token.
pub struct AuthInfo {
    pub accounts: Vec<Account>,
}

impl CfClient {
    /// `GET /user/tokens/verify` — válido solo para tokens *user-owned*.
    async fn verify_user_token(&self) -> Result<TokenVerify> {
        self.get("/user/tokens/verify").await
    }

    /// `GET /accounts/{id}/tokens/verify` — para tokens *account-owned* (`cfat_`).
    async fn verify_account_token(&self, account_id: &str) -> Result<TokenVerify> {
        self.get(&format!("/accounts/{account_id}/tokens/verify"))
            .await
    }

    /// `GET /accounts` — cuentas visibles para el token (ambos tipos).
    pub async fn list_accounts(&self) -> Result<Vec<Account>> {
        self.get("/accounts").await
    }

    /// Verifica la credencial soportando tokens user-owned, account-owned y OAuth.
    ///
    /// Los tokens de cuenta (`cfat_`) NO validan contra `/user/tokens/verify`;
    /// hay que verificarlos contra `/accounts/{id}/tokens/verify`. Descubrimos
    /// la cuenta con `/accounts` (funciona para ambos tipos con scope de cuenta).
    /// Los tokens OAuth tampoco validan contra `tokens/verify`: se usa
    /// `GET /accounts` (scope `account:read`) como prueba de vida.
    pub async fn authenticate(&self) -> Result<AuthInfo> {
        if self.source().is_oauth() {
            let accounts = self.list_accounts().await?;
            if accounts.is_empty() {
                bail!("La sesión OAuth no tiene acceso a ninguna cuenta");
            }
            return Ok(AuthInfo { accounts });
        }

        let user_ok = matches!(self.verify_user_token().await, Ok(v) if v.status == "active");
        let accounts = self.list_accounts().await.unwrap_or_default();

        if user_ok {
            return Ok(AuthInfo { accounts });
        }

        match accounts.first() {
            Some(acc) => {
                let verify = self.verify_account_token(&acc.id).await?;
                if verify.status == "active" {
                    Ok(AuthInfo { accounts })
                } else {
                    bail!("Estado del token: {}", verify.status)
                }
            }
            None => bail!("Token inválido o sin acceso a ninguna cuenta"),
        }
    }
}
