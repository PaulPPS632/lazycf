//! Paleta y estilos. Tema seleccionable en runtime: Cloudflare (default),
//! Everforest y Tokyo Night. Ningún tema pinta el fondo: se respeta el del
//! terminal. Las constantes de antes son ahora accesores que leen el tema
//! activo (`accent()`, `fg()`, …); `border()/title()/selection()` conservan su
//! firma y se apoyan en esos accesores.

use std::sync::atomic::{AtomicUsize, Ordering};

use ratatui::style::{Color, Modifier, Style};

/// Un tema: nombre canónico (persistido en config) + etiqueta visible + los 6
/// colores de la paleta.
pub struct Theme {
    pub name: &'static str,
    pub label: &'static str,
    pub accent: Color,
    pub fg: Color,
    pub dim: Color,
    pub error: Color,
    pub ok: Color,
    pub warn: Color,
}

/// Temas disponibles. El índice 0 es el default (Cloudflare, la "nube naranja").
pub const THEMES: [Theme; 3] = [
    Theme {
        name: "cloudflare",
        label: "Cloudflare",
        accent: Color::Rgb(243, 128, 32),
        fg: Color::Gray,
        dim: Color::DarkGray,
        error: Color::Rgb(220, 80, 80),
        ok: Color::Rgb(120, 200, 120),
        warn: Color::Rgb(220, 190, 90),
    },
    Theme {
        name: "everforest",
        label: "Everforest",
        accent: Color::Rgb(167, 192, 128),
        fg: Color::Rgb(211, 198, 170),
        dim: Color::Rgb(133, 146, 137),
        error: Color::Rgb(230, 126, 128),
        ok: Color::Rgb(131, 192, 146),
        warn: Color::Rgb(219, 188, 127),
    },
    Theme {
        name: "tokyo-night",
        label: "Tokyo Night",
        accent: Color::Rgb(122, 162, 247),
        fg: Color::Rgb(192, 202, 245),
        dim: Color::Rgb(86, 95, 137),
        error: Color::Rgb(247, 118, 142),
        ok: Color::Rgb(158, 206, 106),
        warn: Color::Rgb(224, 175, 104),
    },
];

/// Índice del tema activo. Un solo escritor (event loop), lecturas en render:
/// `Relaxed` basta, no hay orden que preservar entre otras variables.
static CURRENT: AtomicUsize = AtomicUsize::new(0);

/// Fija el tema activo por índice (recortado al rango válido).
pub fn set(idx: usize) {
    CURRENT.store(idx.min(THEMES.len() - 1), Ordering::Relaxed);
}

/// Índice del tema activo.
pub fn current_index() -> usize {
    CURRENT.load(Ordering::Relaxed)
}

/// Tema activo.
pub fn current() -> &'static Theme {
    &THEMES[current_index()]
}

/// Todos los temas disponibles.
pub fn all() -> &'static [Theme] {
    &THEMES
}

/// Índice del tema por nombre canónico (case-insensitive); `None` si no existe.
pub fn index_of(name: &str) -> Option<usize> {
    THEMES
        .iter()
        .position(|t| t.name.eq_ignore_ascii_case(name))
}

// --- Accesores de color (sustituyen a las antiguas constantes) ---

pub fn accent() -> Color {
    current().accent
}
pub fn fg() -> Color {
    current().fg
}
pub fn dim() -> Color {
    current().dim
}
pub fn error() -> Color {
    current().error
}
pub fn ok() -> Color {
    current().ok
}
pub fn warn() -> Color {
    current().warn
}

/// Estilo del borde según foco.
pub fn border(focused: bool) -> Style {
    Style::default().fg(if focused { accent() } else { dim() })
}

/// Estilo del título de un panel según foco.
pub fn title(focused: bool) -> Style {
    let base = Style::default().add_modifier(Modifier::BOLD);
    base.fg(if focused { accent() } else { fg() })
}

/// Estilo del elemento seleccionado en una lista.
pub fn selection() -> Style {
    Style::default().fg(accent()).add_modifier(Modifier::BOLD)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn index_of_matches_case_insensitive() {
        assert_eq!(index_of("cloudflare"), Some(0));
        assert_eq!(index_of("everforest"), Some(1));
        assert_eq!(index_of("tokyo-night"), Some(2));
        assert_eq!(index_of("Everforest"), Some(1));
        assert_eq!(index_of("TOKYO-NIGHT"), Some(2));
        assert_eq!(index_of("desconocido"), None);
    }

    #[test]
    fn default_theme_is_cloudflare_orange() {
        assert_eq!(THEMES[0].name, "cloudflare");
        assert_eq!(THEMES[0].accent, Color::Rgb(243, 128, 32));
    }

    #[test]
    fn set_updates_current_and_clamps() {
        // Estado global de proceso: este es el ÚNICO test que muta `set`; deja
        // el tema en 0 al terminar para no contaminar tests paralelos.
        set(1);
        assert_eq!(current_index(), 1);
        assert_eq!(current().name, "everforest");
        set(99); // fuera de rango → clamp al último
        assert_eq!(current_index(), THEMES.len() - 1);
        set(0);
        assert_eq!(current_index(), 0);
    }
}
