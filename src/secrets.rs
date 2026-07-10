//! Almacenamiento de credenciales de Cloudflare (multi-cuenta).
//!
//! Se guarda una lista JSON de credenciales (API tokens y sesiones OAuth) en
//! una entrada del keychain del OS (Secret Service en Linux).
//! `CLOUDFLARE_API_TOKEN` (env) se añade además, marcada como no persistible.
//! Nunca se guarda en texto plano en disco. Las entradas antiguas (un solo
//! token, o lista de strings) se migran automáticamente.

use color_eyre::eyre::{Result, WrapErr};
use serde::{Deserialize, Serialize};

use crate::oauth::OAuthTokens;

const SERVICE: &str = "lazycf";
/// Entrada actual: JSON `[{"kind":"ApiToken","token":…}, {"kind":"OAuth",…}]`.
/// Antes contenía `["token1", …]` — se migra al cargar.
const TOKENS_ACCOUNT: &str = "cloudflare-api-tokens";
/// Entrada legacy (un solo token) — se migra y se elimina al cargar.
const LEGACY_ACCOUNT: &str = "cloudflare-api-token";
const ENV_VAR: &str = "CLOUDFLARE_API_TOKEN";

/// Credencial de una sesión: API token clásico o tokens OAuth.
// OJO serde: internally-tagged (`tag = "kind"`) no soporta newtype variants de
// primitivos (`ApiToken(String)` falla al serializar en runtime) → struct
// variants.
#[derive(Serialize, Deserialize, Clone)]
#[serde(tag = "kind")]
pub enum Credential {
    ApiToken { token: String },
    OAuth { tokens: OAuthTokens },
}

impl std::fmt::Debug for Credential {
    // Nunca volcar tokens en logs/trazas.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ApiToken { .. } => f.write_str("Credential::ApiToken(<redactado>)"),
            Self::OAuth { tokens } => write!(f, "Credential::OAuth({tokens:?})"),
        }
    }
}

impl Credential {
    /// `true` para credenciales OAuth (refrescables).
    pub fn is_oauth(&self) -> bool {
        matches!(self, Self::OAuth { .. })
    }
}

/// Credencial cargada, con su origen. Las del env no se persisten.
pub struct LoadedCredential {
    pub credential: Credential,
    pub from_env: bool,
}

fn tokens_entry() -> Result<keyring::Entry> {
    keyring::Entry::new(SERVICE, TOKENS_ACCOUNT).wrap_err("abriendo entrada del keyring")
}

fn legacy_entry() -> Result<keyring::Entry> {
    keyring::Entry::new(SERVICE, LEGACY_ACCOUNT).wrap_err("abriendo entrada del keyring")
}

/// Parsea el JSON del keyring aceptando el formato actual (`Vec<Credential>`)
/// y el antiguo (`Vec<String>`, solo API tokens).
fn parse_stored(json: &str) -> Vec<Credential> {
    if let Ok(creds) = serde_json::from_str::<Vec<Credential>>(json) {
        return creds;
    }
    serde_json::from_str::<Vec<String>>(json)
        .map(|tokens| {
            tokens
                .into_iter()
                .map(|token| Credential::ApiToken { token })
                .collect()
        })
        .unwrap_or_default()
}

/// Carga todas las credenciales: env var (primero, no persistible) + keyring.
/// Migra las entradas legacy (token único / lista de strings) al cargar.
pub fn load_credentials() -> Result<Vec<LoadedCredential>> {
    let mut creds: Vec<LoadedCredential> = Vec::new();

    if let Ok(token) = std::env::var(ENV_VAR) {
        let token = token.trim().to_string();
        if !token.is_empty() {
            creds.push(LoadedCredential {
                credential: Credential::ApiToken { token },
                from_env: true,
            });
        }
    }

    // Dedup de API tokens por string (los tokens OAuth rotan: no se dedupean).
    let has_api_token = |creds: &[LoadedCredential], t: &str| {
        creds.iter().any(
            |c| matches!(&c.credential, Credential::ApiToken { token } if token == t),
        )
    };

    match tokens_entry()?.get_password() {
        Ok(json) => {
            for cred in parse_stored(&json) {
                if let Credential::ApiToken { token } = &cred
                    && has_api_token(&creds, token)
                {
                    continue;
                }
                creds.push(LoadedCredential {
                    credential: cred,
                    from_env: false,
                });
            }
        }
        Err(keyring::Error::NoEntry) => {
            // Migración desde la entrada de un solo token.
            if let Ok(legacy) = legacy_entry()?.get_password() {
                if !has_api_token(&creds, &legacy) {
                    creds.push(LoadedCredential {
                        credential: Credential::ApiToken {
                            token: legacy.clone(),
                        },
                        from_env: false,
                    });
                }
                save_credentials(&[Credential::ApiToken { token: legacy }])?;
                let _ = legacy_entry()?.delete_credential();
            }
        }
        Err(e) => return Err(e).wrap_err("leyendo credenciales del keyring"),
    }

    Ok(creds)
}

/// Persiste la lista de credenciales en el keychain (JSON con tag `kind`).
/// El caller debe excluir la credencial del env.
pub fn save_credentials(creds: &[Credential]) -> Result<()> {
    let json = serde_json::to_string(creds).wrap_err("serializando credenciales")?;
    tokens_entry()?
        .set_password(&json)
        .wrap_err("guardando credenciales en el keyring")
}

// --- Credenciales R2 (Access Key + Secret para URLs prefirmadas S3) ---

const R2_ACCOUNT: &str = "r2-credentials";

fn r2_entry() -> Result<keyring::Entry> {
    keyring::Entry::new(SERVICE, R2_ACCOUNT).wrap_err("abriendo entrada R2 del keyring")
}

/// Carga las credenciales R2 `(access_key_id, secret)`. `None` si no hay.
pub fn load_r2_credentials() -> Result<Option<(String, String)>> {
    match r2_entry()?.get_password() {
        Ok(joined) => Ok(joined
            .split_once('\n')
            .map(|(ak, sk)| (ak.to_string(), sk.to_string()))),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(e).wrap_err("leyendo credenciales R2 del keyring"),
    }
}

/// Guarda las credenciales R2 en el keychain (una entrada, `ak\nsk`).
pub fn save_r2_credentials(access_key: &str, secret: &str) -> Result<()> {
    r2_entry()?
        .set_password(&format!("{access_key}\n{secret}"))
        .wrap_err("guardando credenciales R2 en el keyring")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_serde_del_enum_tagged() {
        let creds = vec![
            Credential::ApiToken {
                token: "tok-1".into(),
            },
            Credential::OAuth {
                tokens: OAuthTokens {
                    access_token: "at".into(),
                    refresh_token: "rt".into(),
                    expires_at: 1_700_000_000,
                    scopes: "zone:read".into(),
                },
            },
        ];
        let json = serde_json::to_string(&creds).unwrap();
        assert!(json.contains("\"kind\":\"ApiToken\""), "{json}");
        assert!(json.contains("\"kind\":\"OAuth\""), "{json}");
        let parsed = parse_stored(&json);
        assert_eq!(parsed.len(), 2);
        assert!(matches!(&parsed[0], Credential::ApiToken { token } if token == "tok-1"));
        assert!(
            matches!(&parsed[1], Credential::OAuth { tokens } if tokens.refresh_token == "rt")
        );
    }

    #[test]
    fn migra_la_lista_antigua_de_strings() {
        let parsed = parse_stored(r#"["tok-a","tok-b"]"#);
        assert_eq!(parsed.len(), 2);
        assert!(matches!(&parsed[0], Credential::ApiToken { token } if token == "tok-a"));
        assert!(matches!(&parsed[1], Credential::ApiToken { token } if token == "tok-b"));
    }

    #[test]
    fn json_corrupto_devuelve_lista_vacia() {
        assert!(parse_stored("{no json").is_empty());
        assert!(parse_stored(r#"[{"kind":"Desconocido"}]"#).is_empty());
    }

    #[test]
    fn debug_no_expone_tokens() {
        let c = Credential::ApiToken {
            token: "super-secreto".into(),
        };
        assert!(!format!("{c:?}").contains("super-secreto"));
        let o = Credential::OAuth {
            tokens: OAuthTokens {
                access_token: "at-secreto".into(),
                refresh_token: "rt-secreto".into(),
                expires_at: 0,
                scopes: String::new(),
            },
        };
        let dbg = format!("{o:?}");
        assert!(!dbg.contains("at-secreto") && !dbg.contains("rt-secreto"), "{dbg}");
    }
}
