//! Composición de rectángulos del shell: sidebar + panel principal + barra.

use ratatui::layout::{Constraint, Direction, Layout, Rect};

/// Regiones del shell.
pub struct Shell {
    pub sidebar: Rect,
    pub main: Rect,
    pub command_bar: Rect,
}

/// Divide el área total en sidebar (izq) + main (der) + command bar (abajo).
pub fn shell(area: Rect) -> Shell {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(1)])
        .split(area);
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(26), Constraint::Min(1)])
        .split(rows[0]);
    Shell {
        sidebar: cols[0],
        main: cols[1],
        command_bar: rows[1],
    }
}

/// Divide el área principal en zonas (izq, fija) + registros (resto) para DNS.
pub fn dns_split(main: Rect) -> (Rect, Rect) {
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(30), Constraint::Min(1)])
        .split(main);
    (cols[0], cols[1])
}

/// Rectángulo centrado de tamaño `width` x `height`, recortado al área.
pub fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let w = width.min(area.width);
    let h = height.min(area.height);
    Rect {
        x: area.x + (area.width.saturating_sub(w)) / 2,
        y: area.y + (area.height.saturating_sub(h)) / 2,
        width: w,
        height: h,
    }
}
