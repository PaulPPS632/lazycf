//! Apertura de URLs en el navegador del sistema (xdg-open / open / start).

use color_eyre::eyre::{Result, WrapErr};

/// Página del dashboard para crear API tokens.
pub const TOKEN_PAGE: &str = "https://dash.cloudflare.com/profile/api-tokens";

/// Abre `url` en el navegador por defecto.
pub fn open(url: &str) -> Result<()> {
    ::open::that(url).wrap_err_with(|| format!("abriendo {url} en el navegador"))
}
