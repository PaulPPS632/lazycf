//! Almacenamiento del API token de Cloudflare.
//!
//! Prioridad de lectura: variable de entorno `CLOUDFLARE_API_TOKEN` (útil en
//! CI/headless) y, si no está, el keychain del OS (Secret Service en Linux).
//! Nunca se guarda en texto plano en disco.

use color_eyre::eyre::{Result, WrapErr};

const SERVICE: &str = "lazycf";
const ACCOUNT: &str = "cloudflare-api-token";
const ENV_VAR: &str = "CLOUDFLARE_API_TOKEN";

fn entry() -> Result<keyring::Entry> {
    keyring::Entry::new(SERVICE, ACCOUNT).wrap_err("abriendo entrada del keyring")
}

/// Carga el token: primero env var, luego keyring. `None` si no hay ninguno.
pub fn load_token() -> Result<Option<String>> {
    if let Ok(token) = std::env::var(ENV_VAR) {
        let token = token.trim().to_string();
        if !token.is_empty() {
            return Ok(Some(token));
        }
    }
    match entry()?.get_password() {
        Ok(token) => Ok(Some(token)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(e).wrap_err("leyendo token del keyring"),
    }
}

/// Guarda el token en el keychain del OS.
pub fn save_token(token: &str) -> Result<()> {
    entry()?
        .set_password(token)
        .wrap_err("guardando token en el keyring")
}

/// Borra el token del keychain. No falla si no existía.
#[allow(dead_code)] // usado por el comando "logout" (fase posterior)
pub fn delete_token() -> Result<()> {
    match entry()?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(e).wrap_err("borrando token del keyring"),
    }
}
