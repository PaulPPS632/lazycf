//! Preferencias NO secretas (el token va en el keyring, ver `secrets.rs`).
//! Se serializan a TOML bajo el directorio de config XDG.

use std::fs;
use std::path::PathBuf;

use color_eyre::eyre::{Result, WrapErr};
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Config {
    /// Cuenta por defecto al arrancar (account_id).
    #[serde(default)]
    pub default_account_id: Option<String>,
    /// Zona por defecto al arrancar (zone_id).
    #[serde(default)]
    pub default_zone_id: Option<String>,
    /// Tema activo por nombre canónico (`cloudflare`/`everforest`/`tokyo-night`).
    #[serde(default)]
    pub theme: Option<String>,
}

fn config_path() -> Option<PathBuf> {
    ProjectDirs::from("dev", "lazycf", "lazycf").map(|d| d.config_dir().join("config.toml"))
}

impl Config {
    /// `true` si el archivo de config ya existe (detecta el primer arranque).
    pub fn exists() -> bool {
        config_path().is_some_and(|p| p.exists())
    }

    /// Carga la config; devuelve `Config::default()` si no existe el archivo.
    pub fn load() -> Result<Self> {
        let Some(path) = config_path() else {
            return Ok(Self::default());
        };
        if !path.exists() {
            return Ok(Self::default());
        }
        let raw = fs::read_to_string(&path).wrap_err("leyendo config")?;
        toml::from_str(&raw).wrap_err("parseando config")
    }

    /// Persiste la config, creando el directorio si hace falta.
    pub fn save(&self) -> Result<()> {
        let Some(path) = config_path() else {
            return Ok(());
        };
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).wrap_err("creando dir de config")?;
        }
        let raw = toml::to_string_pretty(self).wrap_err("serializando config")?;
        fs::write(&path, raw).wrap_err("escribiendo config")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_toml_leaves_theme_none() {
        let cfg: Config = toml::from_str("").unwrap();
        assert_eq!(cfg.theme, None);
    }

    #[test]
    fn theme_roundtrips_through_toml() {
        let cfg = Config {
            theme: Some("everforest".into()),
            ..Config::default()
        };
        let raw = toml::to_string_pretty(&cfg).unwrap();
        let back: Config = toml::from_str(&raw).unwrap();
        assert_eq!(back.theme.as_deref(), Some("everforest"));
    }
}
