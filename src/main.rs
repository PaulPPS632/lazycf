//! lazycf — TUI estilo lazygit para administrar Cloudflare.

mod action;
mod api;
mod app;
mod browser;
mod components;
mod config;
mod event;
mod model;
mod secrets;
mod tui;
mod ui;

use std::path::{Path, PathBuf};

use clap::Parser;
use color_eyre::eyre::{Result, WrapErr};

#[derive(Parser, Debug)]
#[command(name = "lazycf", version, about = "TUI para Cloudflare")]
struct Cli {
    /// Escribe logs a este archivo (por defecto, sin logging — la TUI posee stdout).
    #[arg(long, value_name = "FILE")]
    log: Option<PathBuf>,
}

#[tokio::main]
async fn main() -> Result<()> {
    color_eyre::install()?;
    let cli = Cli::parse();
    init_tracing(cli.log.as_deref())?;

    let terminal = tui::init();
    let result = app::App::new()?.run(terminal).await;
    tui::restore();
    result
}

/// Configura `tracing` para escribir a un archivo si se pasó `--log`.
/// No se puede loggear a stdout mientras la TUI posee la terminal.
fn init_tracing(path: Option<&Path>) -> Result<()> {
    if let Some(path) = path {
        let file = std::fs::File::create(path).wrap_err("creando archivo de log")?;
        tracing_subscriber::fmt()
            .with_ansi(false)
            .with_writer(move || file.try_clone().expect("clonando handle de log"))
            .init();
    }
    Ok(())
}
