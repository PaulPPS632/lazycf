//! Vista del módulo Workers: lista de scripts (izq) + detalle con pestañas
//! (Métricas · Implementaciones · Variables · Logs).

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, List, ListItem, ListState, Paragraph, Sparkline, Wrap};
use ratatui::Frame;

use crate::model::{Binding, Deployment, WorkerMetrics, WorkerScript};
use crate::ui::theme;

/// Estado de carga de un dato asíncrono.
#[derive(Default)]
pub enum Loadable<T> {
    #[default]
    Idle,
    Loading,
    Failed,
    Ready(T),
}

impl<T> Loadable<T> {
    pub fn is_idle(&self) -> bool {
        matches!(self, Loadable::Idle)
    }
}

pub const TABS: [&str; 4] = ["Métricas", "Implementaciones", "Variables", "Logs"];

#[derive(Default)]
pub struct WorkersView {
    scripts: Vec<WorkerScript>,
    state: ListState,
    subdomain: Option<String>,
    pub active_tab: usize,
    pub metrics: Loadable<WorkerMetrics>,
    pub deployments: Loadable<Vec<Deployment>>,
    pub bindings: Loadable<Vec<Binding>>,
    /// Índice del binding seleccionado en la pestaña Variables.
    binding_sel: usize,
    /// Líneas del live-tail (más recientes al final).
    logs: Vec<String>,
    /// `true` mientras hay una sesión de tail activa.
    pub tailing: bool,
    pub loading: bool,
    pub error: Option<String>,
}

impl WorkersView {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_empty(&self) -> bool {
        self.scripts.is_empty()
    }

    pub fn reset(&mut self) {
        let subdomain = self.subdomain.take();
        *self = Self::new();
        self.subdomain = subdomain;
    }

    pub fn set_subdomain(&mut self, subdomain: Option<String>) {
        self.subdomain = subdomain;
    }

    pub fn set_scripts(&mut self, scripts: Vec<WorkerScript>) {
        self.scripts = scripts;
        self.loading = false;
        self.error = None;
        self.reset_tabs();
        self.state.select((!self.scripts.is_empty()).then_some(0));
    }

    /// Reinicia los datos de todas las pestañas (al cambiar de script).
    pub fn reset_tabs(&mut self) {
        self.metrics = Loadable::Idle;
        self.deployments = Loadable::Idle;
        self.bindings = Loadable::Idle;
        self.binding_sel = 0;
    }

    pub fn selected(&self) -> Option<&WorkerScript> {
        self.state.selected().and_then(|i| self.scripts.get(i))
    }

    pub fn selected_name(&self) -> Option<String> {
        self.selected().map(|s| s.id.clone())
    }

    pub fn suggested_url(&self) -> Option<String> {
        match (self.selected(), &self.subdomain) {
            (Some(s), Some(sub)) => Some(format!("https://{}.{}.workers.dev", s.id, sub)),
            _ => None,
        }
    }

    pub fn select(&mut self, delta: i32) -> bool {
        let len = self.scripts.len();
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

    pub fn script_at(&mut self, rel: usize) -> bool {
        let idx = rel + self.state.offset();
        if idx >= self.scripts.len() {
            return false;
        }
        let changed = self.state.selected() != Some(idx);
        self.state.select(Some(idx));
        changed
    }

    pub fn set_tab(&mut self, idx: usize) {
        if idx < TABS.len() {
            self.active_tab = idx;
        }
    }

    pub fn cycle_tab(&mut self, delta: i32) {
        let n = TABS.len() as i32;
        self.active_tab = ((((self.active_tab as i32 + delta) % n) + n) % n) as usize;
    }

    // Setters de carga (los llama `app.rs`).
    pub fn begin_metrics(&mut self) {
        self.metrics = Loadable::Loading;
    }
    pub fn set_metrics(&mut self, m: Option<WorkerMetrics>) {
        self.metrics = m.map_or(Loadable::Failed, Loadable::Ready);
    }
    pub fn begin_deployments(&mut self) {
        self.deployments = Loadable::Loading;
    }
    pub fn set_deployments(&mut self, d: Option<Vec<Deployment>>) {
        self.deployments = d.map_or(Loadable::Failed, Loadable::Ready);
    }
    pub fn begin_bindings(&mut self) {
        self.bindings = Loadable::Loading;
    }
    pub fn set_bindings(&mut self, b: Option<Vec<Binding>>) {
        self.bindings = b.map_or(Loadable::Failed, Loadable::Ready);
        self.binding_sel = 0;
    }

    // --- Variables (pestaña 2) ---

    /// `true` si hay bindings cargados y no vacíos (para enrutar ↑↓).
    pub fn bindings_ready_nonempty(&self) -> bool {
        matches!(&self.bindings, Loadable::Ready(b) if !b.is_empty())
    }

    /// Mueve la selección de binding (envuelve). Devuelve si cambió.
    pub fn select_binding(&mut self, delta: i32) -> bool {
        let Loadable::Ready(bs) = &self.bindings else {
            return false;
        };
        let len = bs.len();
        if len == 0 {
            return false;
        }
        let cur = self.binding_sel.min(len - 1) as i32;
        let n = len as i32;
        let next = ((((cur + delta) % n) + n) % n) as usize;
        let changed = next != self.binding_sel;
        self.binding_sel = next;
        changed
    }

    pub fn selected_binding(&self) -> Option<&Binding> {
        let Loadable::Ready(bs) = &self.bindings else {
            return None;
        };
        bs.get(self.binding_sel.min(bs.len().saturating_sub(1)))
    }

    // --- Logs / live-tail (pestaña 3) ---

    pub fn set_tailing(&mut self, on: bool) {
        self.tailing = on;
    }

    pub fn clear_logs(&mut self) {
        self.logs.clear();
    }

    /// Añade líneas al buffer de logs, acotado a las últimas 1000.
    pub fn push_logs(&mut self, mut lines: Vec<String>) {
        self.logs.append(&mut lines);
        const CAP: usize = 1000;
        if self.logs.len() > CAP {
            let drop = self.logs.len() - CAP;
            self.logs.drain(0..drop);
        }
    }

    // --- Render ---

    pub fn draw_list(&mut self, frame: &mut Frame, area: Rect, focused: bool) {
        let block = Block::bordered()
            .title(" Workers ")
            .border_style(theme::border(focused))
            .title_style(theme::title(focused));

        if self.loading {
            frame.render_widget(placeholder("Cargando workers…", block), area);
            return;
        }
        if let Some(err) = &self.error {
            frame.render_widget(placeholder(&format!("✗ {err}"), block), area);
            return;
        }
        if self.scripts.is_empty() {
            frame.render_widget(placeholder("Sin workers en esta cuenta", block), area);
            return;
        }

        let items: Vec<ListItem> = self
            .scripts
            .iter()
            .map(|s| ListItem::new(s.id.clone()))
            .collect();
        let list = List::new(items)
            .block(block)
            .highlight_style(theme::selection())
            .highlight_symbol("▶ ");
        frame.render_stateful_widget(list, area, &mut self.state);
    }

    pub fn draw_detail(&self, frame: &mut Frame, area: Rect, focused: bool) {
        let title = match self.selected() {
            Some(s) if !s.modified_on.is_empty() => {
                format!(" {} · mod {} ", s.id, short_date(&s.modified_on))
            }
            Some(s) => format!(" {} ", s.id),
            None => " Detalle ".to_string(),
        };
        let block = Block::bordered()
            .title(title)
            .border_style(theme::border(focused))
            .title_style(theme::title(focused));
        let inner = block.inner(area);
        frame.render_widget(block, area);

        if self.selected().is_none() {
            frame.render_widget(
                Paragraph::new("Selecciona un worker").style(Style::default().fg(theme::DIM)),
                inner,
            );
            return;
        }

        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(1), Constraint::Length(1), Constraint::Min(0)])
            .split(inner);
        frame.render_widget(self.tab_bar(), rows[0]);
        // rows[1] queda como separador visual.

        match self.active_tab {
            0 => self.draw_metrics(frame, rows[2]),
            1 => self.draw_deployments(frame, rows[2]),
            2 => self.draw_bindings(frame, rows[2]),
            _ => self.draw_logs(frame, rows[2]),
        }
    }

    fn tab_bar(&self) -> Line<'static> {
        let mut spans = Vec::new();
        for (i, t) in TABS.iter().enumerate() {
            let style = if i == self.active_tab {
                Style::default()
                    .fg(theme::ACCENT)
                    .add_modifier(ratatui::style::Modifier::BOLD)
            } else {
                Style::default().fg(theme::DIM)
            };
            spans.push(Span::styled(format!(" {} {} ", i + 1, t), style));
            if i + 1 < TABS.len() {
                spans.push(Span::styled("·", Style::default().fg(theme::DIM)));
            }
        }
        Line::from(spans)
    }

    fn draw_metrics(&self, frame: &mut Frame, area: Rect) {
        match &self.metrics {
            Loadable::Idle | Loadable::Loading => {
                frame.render_widget(dim("Cargando métricas…"), area);
            }
            Loadable::Failed => {
                frame.render_widget(dim("Métricas no disponibles"), area);
            }
            Loadable::Ready(m) => {
                let split = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([Constraint::Min(6), Constraint::Length(5)])
                    .split(area);
                let rate = m.error_rate();
                let lines = vec![
                    metric_line("Requests", &m.requests.to_string()),
                    metric_line("Errores", &m.errors.to_string()),
                    Line::from(vec![
                        Span::styled(format!("{:<12}", "Tasa error"), Style::default().fg(theme::DIM)),
                        Span::styled(format!("{rate:.2}%"), Style::default().fg(rate_color(rate))),
                    ]),
                    metric_line("CPU p50", &format!("{:.0} µs", m.cpu_p50)),
                    metric_line("CPU p99", &format!("{:.0} µs", m.cpu_p99)),
                    Line::from(Span::styled(
                        "requests / hora (24h):",
                        Style::default().fg(theme::DIM),
                    )),
                ];
                frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: true }), split[0]);
                if m.series.is_empty() {
                    frame.render_widget(dim("(sin datos)"), split[1]);
                } else {
                    let spark = Sparkline::default()
                        .data(&m.series)
                        .style(Style::default().fg(theme::ACCENT));
                    frame.render_widget(spark, split[1]);
                }
            }
        }
    }

    fn draw_deployments(&self, frame: &mut Frame, area: Rect) {
        let lines: Vec<Line> = match &self.deployments {
            Loadable::Idle | Loadable::Loading => vec![dim_line("Cargando…")],
            Loadable::Failed => vec![dim_line("No disponible")],
            Loadable::Ready(d) if d.is_empty() => vec![dim_line("Sin implementaciones")],
            Loadable::Ready(d) => d
                .iter()
                .map(|dep| {
                    Line::from(vec![
                        Span::styled("▪ ", Style::default().fg(theme::ACCENT)),
                        Span::raw(short_date(&dep.created_on)),
                        Span::styled("  ", Style::default()),
                        Span::styled(dep.author_email.clone(), Style::default().fg(theme::FG)),
                        Span::styled(
                            format!("  ({})", dep.source),
                            Style::default().fg(theme::DIM),
                        ),
                    ])
                })
                .collect(),
        };
        frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: true }), area);
    }

    fn draw_bindings(&self, frame: &mut Frame, area: Rect) {
        let lines: Vec<Line> = match &self.bindings {
            Loadable::Idle | Loadable::Loading => vec![dim_line("Cargando…")],
            Loadable::Failed => vec![dim_line("No disponible")],
            Loadable::Ready(b) if b.is_empty() => vec![dim_line("Sin variables")],
            Loadable::Ready(b) => {
                let sel = self.binding_sel.min(b.len() - 1);
                let mut ls: Vec<Line> = b
                    .iter()
                    .enumerate()
                    .map(|(i, bind)| {
                        let selected = i == sel;
                        let marker = if selected { "▶ " } else { "  " };
                        let name_style = if selected {
                            Style::default()
                                .fg(theme::ACCENT)
                                .add_modifier(ratatui::style::Modifier::BOLD)
                        } else {
                            Style::default().fg(theme::ACCENT)
                        };
                        let value_style = if bind.is_secret() {
                            Style::default().fg(theme::WARN)
                        } else {
                            Style::default().fg(theme::FG)
                        };
                        Line::from(vec![
                            Span::styled(format!("{marker}{:<20}", bind.name), name_style),
                            Span::styled(format!("{:<12}", bind.btype), Style::default().fg(theme::DIM)),
                            Span::styled(bind.display_value(), value_style),
                        ])
                    })
                    .collect();
                ls.push(Line::from(""));
                ls.push(dim_line("e editar · a añadir secreto"));
                ls
            }
        };
        frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: true }), area);
    }

    fn draw_logs(&self, frame: &mut Frame, area: Rect) {
        if self.logs.is_empty() && !self.tailing {
            let lines = vec![
                dim_line("Live-tail de logs vía WebSocket (trace-v1)."),
                dim_line("Pulsa 'l' para iniciar/detener el streaming."),
            ];
            frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: true }), area);
            return;
        }
        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(1), Constraint::Min(0)])
            .split(area);
        let header = if self.tailing {
            Span::styled("● en vivo · 'l' para detener", Style::default().fg(theme::OK))
        } else {
            Span::styled("○ detenido · 'l' para iniciar", Style::default().fg(theme::DIM))
        };
        frame.render_widget(Paragraph::new(Line::from(header)), rows[0]);

        // Muestra solo las últimas líneas que caben (auto-scroll al final).
        let cap = rows[1].height as usize;
        let start = self.logs.len().saturating_sub(cap);
        let lines: Vec<Line> = self.logs[start..].iter().map(|l| log_line(l)).collect();
        frame.render_widget(Paragraph::new(lines), rows[1]);
    }
}

/// Colorea una línea de log según su tipo (cabecera / error / normal).
fn log_line(s: &str) -> Line<'static> {
    let style = if s.contains('✗') || s.contains("[error]") {
        Style::default().fg(theme::ERROR)
    } else if s.trim_start().starts_with('▪') {
        Style::default().fg(theme::ACCENT)
    } else {
        Style::default().fg(theme::FG)
    };
    Line::from(Span::styled(s.to_string(), style))
}

/// Divide el área principal en lista (izq) + detalle (der).
pub fn split(main: Rect) -> (Rect, Rect) {
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(28), Constraint::Min(1)])
        .split(main);
    (cols[0], cols[1])
}

fn metric_line(label: &str, value: &str) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("{label:<12}"), Style::default().fg(theme::DIM)),
        Span::styled(value.to_string(), Style::default().fg(theme::FG)),
    ])
}

fn rate_color(rate: f64) -> ratatui::style::Color {
    if rate < 1.0 {
        theme::OK
    } else if rate < 5.0 {
        theme::WARN
    } else {
        theme::ERROR
    }
}

fn short_date(iso: &str) -> String {
    if iso.len() >= 16 {
        iso[..16].replace('T', " ")
    } else {
        iso.to_string()
    }
}

fn dim(text: &str) -> Paragraph<'_> {
    Paragraph::new(text)
        .style(Style::default().fg(theme::DIM))
        .wrap(Wrap { trim: true })
}

fn dim_line(text: &str) -> Line<'static> {
    Line::from(Span::styled(text.to_string(), Style::default().fg(theme::DIM)))
}

fn placeholder<'a>(text: &'a str, block: Block<'a>) -> Paragraph<'a> {
    Paragraph::new(text)
        .block(block)
        .style(Style::default().fg(theme::DIM))
        .wrap(Wrap { trim: true })
}
