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
}

fn config_path() -> Option<PathBuf> {
    ProjectDirs::from("dev", "lazycf", "lazycf").map(|d| d.config_dir().join("config.toml"))
}

impl Config {
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
    #[allow(dead_code)] // se usará al guardar cuenta/zona por defecto (Fase 1)
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
