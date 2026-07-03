//! Vista del módulo Túneles: lista de túneles (izq) + detalle (der) con
//! estado, reglas de ingress y conexiones activas.

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, List, ListItem, ListState, Paragraph, Wrap};
use ratatui::Frame;

use crate::model::{IngressRule, Tunnel};
use crate::ui::theme;

#[derive(Default)]
pub struct TunnelsView {
    tunnels: Vec<Tunnel>,
    state: ListState,
    ingress: Vec<IngressRule>,
    pub loading: bool,
    pub loading_ingress: bool,
    pub error: Option<String>,
}

impl TunnelsView {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_empty(&self) -> bool {
        self.tunnels.is_empty()
    }

    pub fn reset(&mut self) {
        *self = Self::new();
    }

    pub fn set_tunnels(&mut self, tunnels: Vec<Tunnel>) {
        self.tunnels = tunnels;
        self.loading = false;
        self.error = None;
        self.ingress.clear();
        self.state
            .select(if self.tunnels.is_empty() { None } else { Some(0) });
    }

    pub fn selected(&self) -> Option<&Tunnel> {
        self.state.selected().and_then(|i| self.tunnels.get(i))
    }

    pub fn selected_id(&self) -> Option<String> {
        self.selected().map(|t| t.id.clone())
    }

    /// Selecciona un túnel por fila relativa (click); `true` si cambió.
    pub fn tunnel_at(&mut self, rel: usize) -> bool {
        let idx = rel + self.state.offset();
        if idx >= self.tunnels.len() {
            return false;
        }
        let changed = self.state.selected() != Some(idx);
        self.state.select(Some(idx));
        changed
    }

    /// Mueve la selección; `true` si cambió.
    pub fn select(&mut self, delta: i32) -> bool {
        let len = self.tunnels.len();
        if len == 0 {
            return false;
        }
        let cur = self.state.selected().unwrap_or(0) as i32;
        let n = len as i32;
        let next = (((cur + delta) % n) + n) % n;
        let changed = next != cur;
        self.state.select(Some(next as usize));
        changed
    }

    pub fn begin_loading_ingress(&mut self) {
        self.loading_ingress = true;
        self.ingress.clear();
    }

    pub fn set_ingress(&mut self, rules: Vec<IngressRule>) {
        self.ingress = rules;
        self.loading_ingress = false;
    }

    // --- Render ---

    pub fn draw_list(&mut self, frame: &mut Frame, area: Rect, focused: bool) {
        let block = Block::bordered()
            .title(" Túneles ")
            .border_style(theme::border(focused))
            .title_style(theme::title(focused));

        if self.loading {
            frame.render_widget(placeholder("Cargando túneles…", block), area);
            return;
        }
        if let Some(err) = &self.error {
            frame.render_widget(placeholder(&format!("✗ {err}"), block), area);
            return;
        }
        if self.tunnels.is_empty() {
            frame.render_widget(placeholder("Sin túneles en esta cuenta", block), area);
            return;
        }

        let items: Vec<ListItem> = self
            .tunnels
            .iter()
            .map(|t| {
                ListItem::new(Line::from(vec![
                    Span::styled("● ", Style::default().fg(status_color(&t.status))),
                    Span::raw(t.name.clone()),
                ]))
            })
            .collect();
        let list = List::new(items)
            .block(block)
            .highlight_style(theme::selection())
            .highlight_symbol("▶ ");
        frame.render_stateful_widget(list, area, &mut self.state);
    }

    pub fn draw_detail(&self, frame: &mut Frame, area: Rect, focused: bool) {
        let block = Block::bordered()
            .title(" Detalle ")
            .border_style(theme::border(focused))
            .title_style(theme::title(focused));

        let Some(tunnel) = self.selected() else {
            frame.render_widget(placeholder("Selecciona un túnel", block), area);
            return;
        };

        let mut lines: Vec<Line> = Vec::new();
        lines.push(Line::from(vec![
            Span::styled("Estado: ", Style::default().fg(theme::DIM)),
            Span::styled(
                status_label(&tunnel.status),
                Style::default().fg(status_color(&tunnel.status)),
            ),
        ]));
        lines.push(Line::from(vec![
            Span::styled("ID: ", Style::default().fg(theme::DIM)),
            Span::raw(tunnel.id.clone()),
        ]));
        lines.push(Line::from(""));

        // Conexiones (embebidas en el listado).
        lines.push(Line::from(Span::styled(
            format!("Conexiones activas: {}", tunnel.connections.len()),
            theme::title(false),
        )));
        for c in &tunnel.connections {
            let mut parts = vec![format!("  · {}", c.colo_name)];
            if !c.origin_ip.is_empty() {
                parts.push(c.origin_ip.clone());
            }
            if !c.client_version.is_empty() {
                parts.push(format!("v{}", c.client_version));
            }
            lines.push(Line::from(Span::styled(
                parts.join("  "),
                Style::default().fg(theme::FG),
            )));
        }
        lines.push(Line::from(""));

        // Ingress (hostname → servicio local).
        lines.push(Line::from(Span::styled("Rutas (ingress):", theme::title(false))));
        if self.loading_ingress {
            lines.push(Line::from(Span::styled(
                "  Cargando…",
                Style::default().fg(theme::DIM),
            )));
        } else if self.ingress.is_empty() {
            lines.push(Line::from(Span::styled(
                "  (sin config remota / túnel local)",
                Style::default().fg(theme::DIM),
            )));
        } else {
            for r in &self.ingress {
                let mut host = if r.hostname.is_empty() {
                    "*".to_string()
                } else {
                    r.hostname.clone()
                };
                if let Some(path) = &r.path {
                    host.push_str(path);
                }
                lines.push(Line::from(vec![
                    Span::styled(format!("  {host}"), Style::default().fg(theme::ACCENT)),
                    Span::styled("  →  ", Style::default().fg(theme::DIM)),
                    Span::raw(r.service.clone()),
                ]));
            }
        }

        frame.render_widget(
            Paragraph::new(lines).block(block).wrap(Wrap { trim: true }),
            area,
        );
    }
}

/// Divide el área principal en lista (izq) + detalle (der).
pub fn split(main: Rect) -> (Rect, Rect) {
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(34), Constraint::Min(1)])
        .split(main);
    (cols[0], cols[1])
}

fn status_color(status: &str) -> ratatui::style::Color {
    match status {
        "healthy" => theme::OK,
        "degraded" => theme::WARN,
        "down" => theme::ERROR,
        _ => theme::DIM,
    }
}

fn status_label(status: &str) -> String {
    match status {
        "healthy" => "● activo (healthy)",
        "degraded" => "● degradado",
        "down" => "● caído",
        "inactive" => "○ inactivo",
        other => other,
    }
    .to_string()
}

fn placeholder<'a>(text: &'a str, block: Block<'a>) -> Paragraph<'a> {
    Paragraph::new(text)
        .block(block)
        .style(Style::default().fg(theme::DIM))
        .wrap(Wrap { trim: true })
}
