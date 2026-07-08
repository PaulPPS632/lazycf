//! Vista del módulo Túneles, estilo lazygit:
//!   col.2 = lista de túneles (arriba) / detalle: estado, ID, conexiones (abajo)
//!   col.3 = rutas del túnel (ingress) navegables y editables.

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, List, ListItem, ListState, Paragraph, Wrap};
use ratatui::Frame;

use crate::model::{IngressRule, Tunnel};
use crate::ui::theme;
use crate::ui::widgets::{placeholder, row_at, select_wrap};

#[derive(Default)]
pub struct TunnelsView {
    tunnels: Vec<Tunnel>,
    state: ListState,
    ingress: Vec<IngressRule>,
    /// Selección dentro de las rutas con hostname (col. 3).
    route_state: ListState,
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
        // Activos primero, luego alfabético — lo importante arriba.
        self.tunnels.sort_by(|a, b| {
            status_rank(&a.status)
                .cmp(&status_rank(&b.status))
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });
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
        row_at(&mut self.state, self.tunnels.len(), rel)
    }

    /// Mueve la selección; `true` si cambió.
    pub fn select(&mut self, delta: i32) -> bool {
        select_wrap(&mut self.state, self.tunnels.len(), delta)
    }

    pub fn begin_loading_ingress(&mut self) {
        self.loading_ingress = true;
        self.ingress.clear();
        self.route_state.select(None);
    }

    pub fn set_ingress(&mut self, rules: Vec<IngressRule>) {
        self.ingress = rules;
        self.loading_ingress = false;
        let has = !self.hostname_rules().is_empty();
        self.route_state.select(has.then_some(0));
    }

    // --- Rutas (col. 3) ---

    /// Reglas de ingress con hostname (las editables; excluye la catch-all).
    fn hostname_rules(&self) -> Vec<&IngressRule> {
        self.ingress.iter().filter(|r| !r.hostname.is_empty()).collect()
    }

    /// Regla de ingress seleccionada en la lista de rutas.
    pub fn selected_route(&self) -> Option<IngressRule> {
        let rules = self.hostname_rules();
        self.route_state
            .selected()
            .and_then(|i| rules.get(i).map(|r| (*r).clone()))
    }

    /// Mueve la selección de ruta; `true` si cambió.
    pub fn select_route(&mut self, delta: i32) -> bool {
        let len = self.hostname_rules().len();
        select_wrap(&mut self.route_state, len, delta)
    }

    /// Selecciona una ruta por fila relativa (click); `true` si cambió.
    pub fn route_at(&mut self, rel: usize) -> bool {
        let len = self.hostname_rules().len();
        row_at(&mut self.route_state, len, rel)
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
            // Ojo: sin el permiso "Cloudflare Tunnel", el API devuelve lista
            // vacía (200) en lugar de 403 — no se puede distinguir del caso real.
            frame.render_widget(
                placeholder(
                    "Sin túneles en esta cuenta.\n\n¿Esperabas ver túneles? Comprueba que el \
                     token tenga el permiso Cuenta → Cloudflare Tunnel (sin él, el API \
                     devuelve una lista vacía).",
                    block,
                ),
                area,
            );
            return;
        }

        // Título con contador: ↑ = healthy/degraded, ↓ = el resto.
        let up = self
            .tunnels
            .iter()
            .filter(|t| status_rank(&t.status) <= 1)
            .count();
        let down = self.tunnels.len() - up;
        let block = Block::bordered()
            .title(Line::from(vec![
                Span::styled(" Túneles (", theme::title(focused)),
                Span::styled(format!("{up}↑"), Style::default().fg(theme::OK)),
                Span::styled(" · ", Style::default().fg(theme::DIM)),
                Span::styled(format!("{down}↓"), Style::default().fg(theme::ERROR)),
                Span::styled(") ", theme::title(focused)),
            ]))
            .border_style(theme::border(focused));

        // Ancho útil por fila: bordes (2) + símbolo de selección "▶ " (2).
        let width = area.width.saturating_sub(4) as usize;
        let items: Vec<ListItem> = self
            .tunnels
            .iter()
            .map(|t| {
                let (tag, color) = status_tag(&t.status);
                let tag_w = tag.chars().count();
                // "● " (2) + nombre + ≥1 espacio + tag pegado a la derecha.
                let avail = width.saturating_sub(2 + tag_w + 1);
                let mut name = t.name.clone();
                if name.chars().count() > avail {
                    name = name.chars().take(avail.saturating_sub(1)).collect();
                    name.push('…');
                }
                let pad = width.saturating_sub(2 + name.chars().count() + tag_w);
                ListItem::new(Line::from(vec![
                    Span::styled("● ", Style::default().fg(status_color(&t.status))),
                    Span::raw(name),
                    Span::raw(" ".repeat(pad)),
                    Span::styled(tag, Style::default().fg(color).add_modifier(Modifier::BOLD)),
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
        // Las rutas viven en la col. 3; aquí solo el conteo.
        lines.push(Line::from(Span::styled(
            format!("Rutas: {}", self.hostname_rules().len()),
            theme::title(false),
        )));

        frame.render_widget(
            Paragraph::new(lines).block(block).wrap(Wrap { trim: true }),
            area,
        );
    }

    /// Col. 3: rutas del túnel (ingress) navegables.
    pub fn draw_routes(&mut self, frame: &mut Frame, area: Rect, focused: bool) {
        let rules = self.hostname_rules();
        let block = Block::bordered()
            .title(format!(" Rutas ({}) ", rules.len()))
            .border_style(theme::border(focused))
            .title_style(theme::title(focused));

        if self.loading_ingress {
            frame.render_widget(placeholder("Cargando rutas…", block), area);
            return;
        }
        if self.selected().is_none() {
            frame.render_widget(placeholder("Selecciona un túnel", block), area);
            return;
        }
        if rules.is_empty() {
            frame.render_widget(
                placeholder(
                    "Sin rutas públicas.\n\nPulsa 'a' para añadir una (hostname → servicio). \
                     Solo túneles gestionados en Cloudflare; los locales guardan su config \
                     en el propio host.",
                    block,
                ),
                area,
            );
            return;
        }

        let items: Vec<ListItem> = rules
            .iter()
            .map(|r| {
                let mut host = r.hostname.clone();
                if let Some(path) = &r.path {
                    host.push_str(path);
                }
                ListItem::new(Line::from(vec![
                    Span::styled(host, Style::default().fg(theme::ACCENT)),
                    Span::styled("  →  ", Style::default().fg(theme::DIM)),
                    Span::styled(r.service.clone(), Style::default().fg(theme::FG)),
                ]))
            })
            .collect();
        let list = List::new(items)
            .block(block)
            .highlight_style(theme::selection())
            .highlight_symbol("▶ ");
        frame.render_stateful_widget(list, area, &mut self.route_state);
    }
}

/// Divide el área principal: col.2 = túneles (arriba) + detalle (abajo),
/// col.3 = rutas del túnel.
pub fn split(main: Rect) -> (Rect, Rect, Rect) {
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(38), Constraint::Min(1)])
        .split(main);
    let left = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(55), Constraint::Min(1)])
        .split(cols[0]);
    (left[0], left[1], cols[1])
}

/// Orden de la lista: activos primero.
fn status_rank(status: &str) -> u8 {
    match status {
        "healthy" => 0,
        "degraded" => 1,
        "down" => 2,
        "inactive" => 3,
        _ => 4,
    }
}

/// Etiqueta corta y coloreada para la lista.
fn status_tag(status: &str) -> (String, ratatui::style::Color) {
    match status {
        "healthy" => ("ACTIVO".into(), theme::OK),
        "degraded" => ("DEGRADADO".into(), theme::WARN),
        "down" => ("CAÍDO".into(), theme::ERROR),
        "inactive" => ("INACTIVO".into(), theme::DIM),
        other => (other.to_uppercase(), theme::DIM),
    }
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
