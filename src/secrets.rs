//! Almacenamiento de los API tokens de Cloudflare (multi-cuenta).
//!
//! Se guarda una lista JSON de tokens en una entrada del keychain del OS
//! (Secret Service en Linux). `CLOUDFLARE_API_TOKEN` (env) se añade además,
//! sin persistirse. Nunca se guarda en texto plano en disco. La entrada
//! antigua de un solo token se migra automáticamente a la lista.

use color_eyre::eyre::{Result, WrapErr};

const SERVICE: &str = "lazycf";
/// Entrada nueva: JSON `["token1", "token2", …]`.
const TOKENS_ACCOUNT: &str = "cloudflare-api-tokens";
/// Entrada legacy (un solo token) — se migra y se elimina al cargar.
const LEGACY_ACCOUNT: &str = "cloudflare-api-token";
const ENV_VAR: &str = "CLOUDFLARE_API_TOKEN";

fn tokens_entry() -> Result<keyring::Entry> {
    keyring::Entry::new(SERVICE, TOKENS_ACCOUNT).wrap_err("abriendo entrada del keyring")
}

fn legacy_entry() -> Result<keyring::Entry> {
    keyring::Entry::new(SERVICE, LEGACY_ACCOUNT).wrap_err("abriendo entrada del keyring")
}

/// Carga todos los tokens: env var (primero, sin persistir) + keyring.
/// Migra la entrada legacy de un solo token a la lista si existe.
pub fn load_tokens() -> Result<Vec<String>> {
    let mut tokens: Vec<String> = Vec::new();

    if let Ok(token) = std::env::var(ENV_VAR) {
        let token = token.trim().to_string();
        if !token.is_empty() {
            tokens.push(token);
        }
    }

    // Lista nueva (JSON).
    match tokens_entry()?.get_password() {
        Ok(json) => {
            let stored: Vec<String> = serde_json::from_str(&json).unwrap_or_default();
            for t in stored {
                if !tokens.contains(&t) {
                    tokens.push(t);
                }
            }
        }
        Err(keyring::Error::NoEntry) => {
            // Migración desde la entrada de un solo token.
            if let Ok(legacy) = legacy_entry()?.get_password() {
                if !tokens.contains(&legacy) {
                    tokens.push(legacy.clone());
                }
                save_tokens(std::slice::from_ref(&legacy))?;
                let _ = legacy_entry()?.delete_credential();
            }
        }
        Err(e) => return Err(e).wrap_err("leyendo tokens del keyring"),
    }

    Ok(tokens)
}

/// Persiste la lista de tokens en el keychain (JSON).
pub fn save_tokens(tokens: &[String]) -> Result<()> {
    let json = serde_json::to_string(tokens).wrap_err("serializando tokens")?;
    tokens_entry()?
        .set_password(&json)
        .wrap_err("guardando tokens en el keyring")
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
