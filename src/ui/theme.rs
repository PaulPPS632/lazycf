//! Paleta y estilos. Naranja Cloudflare para acentos y foco.

use ratatui::style::{Color, Modifier, Style};

/// Naranja Cloudflare (la "nube naranja").
pub const ACCENT: Color = Color::Rgb(243, 128, 32);
pub const FG: Color = Color::Gray;
pub const DIM: Color = Color::DarkGray;
pub const ERROR: Color = Color::Rgb(220, 80, 80);
pub const OK: Color = Color::Rgb(120, 200, 120);
pub const WARN: Color = Color::Rgb(220, 190, 90);

/// Estilo del borde según foco.
pub fn border(focused: bool) -> Style {
    Style::default().fg(if focused { ACCENT } else { DIM })
}

/// Estilo del título de un panel según foco.
pub fn title(focused: bool) -> Style {
    let base = Style::default().add_modifier(Modifier::BOLD);
    base.fg(if focused { ACCENT } else { FG })
}

/// Estilo del elemento seleccionado en una lista.
pub fn selection() -> Style {
    Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)
}
