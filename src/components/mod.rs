//! Componentes de UI (patrón Component, análogo a los contexts/controllers de
//! lazygit): cada panel encapsula su estado, input y render.

pub mod command_bar;
pub mod config;
pub mod d1;
pub mod detail;
pub mod dns;
pub mod input;
pub mod popup;
pub mod queues;
pub mod r2;
pub mod sidebar;
pub mod tunnels;
pub mod welcome;
pub mod workers;

use crossterm::event::KeyEvent;
use ratatui::Frame;
use ratatui::layout::Rect;

use crate::action::Action;

/// Un panel enfocable que maneja teclas y se dibuja.
pub trait Component {
    /// Procesa una tecla; devuelve una `Action` si genera una intención.
    fn handle_key(&mut self, _key: KeyEvent) -> Option<Action> {
        None
    }
    /// Dibuja el componente en `area`. `focused` ajusta el estilo.
    fn draw(&mut self, frame: &mut Frame, area: Rect, focused: bool);
}

/// Los 6 módulos de Cloudflare que expone la TUI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Module {
    Dns,
    Tunnels,
    Workers,
    Queues,
    D1,
    R2,
}

impl Module {
    pub const ALL: [Module; 6] = [
        Module::Dns,
        Module::Tunnels,
        Module::Workers,
        Module::Queues,
        Module::D1,
        Module::R2,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Module::Dns => "DNS y Dominios",
            Module::Tunnels => "Túneles",
            Module::Workers => "Workers",
            Module::Queues => "Queues",
            Module::D1 => "D1",
            Module::R2 => "R2",
        }
    }

    pub fn icon(self) -> &'static str {
        match self {
            Module::Dns => "🌐",
            Module::Tunnels => "🚇",
            Module::Workers => "⚙",
            Module::Queues => "📨",
            Module::D1 => "🗄",
            Module::R2 => "📦",
        }
    }

    /// Descripción corta del módulo (placeholder hasta implementar cada fase).
    pub fn hint(self) -> &'static str {
        match self {
            Module::Dns => "Zonas y registros DNS · toggle de proxy · purgar caché.",
            Module::Tunnels => "Túneles Zero Trust · estado en vivo · rutas ingress.",
            Module::Workers => "Scripts · logs en vivo · métricas · testing de rutas.",
            Module::Queues => "Colas · mensajes · consumers · backlog · purgar.",
            Module::D1 => "Esquema · consola SQL · backups (export/import).",
            Module::R2 => "Buckets · navegador de objetos · URLs firmadas.",
        }
    }
}
