//! Vista del módulo Workers: lista de scripts (izq) + detalle con pestañas
//! (Métricas · Implementaciones · Variables · Logs).

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, List, ListItem, ListState, Paragraph, Sparkline, Wrap};
use ratatui::Frame;

use crate::api::workers::TailEvent;
use crate::components::input::TextInput;
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

pub const TABS: [&str; 5] = [
    "Métricas",
    "Implementaciones",
    "Variables",
    "Logs",
    "Rutas",
];

/// Rutas de zona + custom domains que apuntan a un Worker.
#[derive(Debug, Clone, Default)]
pub struct RoutingInfo {
    /// (patrón de ruta, nombre de zona).
    pub routes: Vec<(String, String)>,
    /// Hostnames de custom domains.
    pub domains: Vec<String>,
}

#[derive(Default)]
pub struct WorkersView {
    scripts: Vec<WorkerScript>,
    state: ListState,
    subdomain: Option<String>,
    pub active_tab: usize,
    pub metrics: Loadable<WorkerMetrics>,
    pub deployments: Loadable<Vec<Deployment>>,
    pub bindings: Loadable<Vec<Binding>>,
    pub routing: Loadable<RoutingInfo>,
    /// Nº de zonas consultadas para las rutas (aviso durante la carga).
    routing_zones: usize,
    /// Índice del binding seleccionado en la pestaña Variables.
    binding_sel: usize,
    /// Selección + scroll de la pestaña Implementaciones.
    deploy_state: ListState,
    /// Eventos del live-tail (más recientes al final).
    logs: Vec<TailEvent>,
    /// Selección + scroll del panel de logs (indexa sobre `log_visible`).
    log_state: ListState,
    /// Auto-seguimiento del último evento (se pausa al navegar hacia atrás).
    log_follow: bool,
    /// Índices de `logs` que pasan el filtro actual.
    log_visible: Vec<usize>,
    /// Filtro de texto de los logs (tecla `/`).
    log_filter: TextInput,
    /// Mostrar solo eventos con error (tecla `E`).
    log_errors_only: bool,
    /// `true` mientras hay una sesión de tail activa.
    pub tailing: bool,
    pub loading: bool,
    pub error: Option<String>,
}

impl WorkersView {
    pub fn new() -> Self {
        Self {
            // `log_follow` arranca en `true`: por defecto seguimos el final.
            log_follow: true,
            ..Default::default()
        }
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
        self.routing = Loadable::Idle;
        self.binding_sel = 0;
        self.deploy_state.select(None);
    }

    pub fn selected(&self) -> Option<&WorkerScript> {
        self.state.selected().and_then(|i| self.scripts.get(i))
    }

    pub fn selected_name(&self) -> Option<String> {
        self.selected().map(|s| s.id.clone())
    }

    /// Selecciona el script por nombre (salto desde otro módulo, p. ej. Queues
    /// hacia el tail del consumer). `true` si el script está en la lista.
    pub fn select_by_name(&mut self, name: &str) -> bool {
        match self.scripts.iter().position(|s| s.id == name) {
            Some(idx) => {
                self.state.select(Some(idx));
                true
            }
            None => false,
        }
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
        let nonempty = matches!(&d, Some(v) if !v.is_empty());
        self.deployments = d.map_or(Loadable::Failed, Loadable::Ready);
        // El índice 0 es el deployment activo.
        self.deploy_state.select(nonempty.then_some(0));
    }
    pub fn begin_bindings(&mut self) {
        self.bindings = Loadable::Loading;
    }
    pub fn set_bindings(&mut self, b: Option<Vec<Binding>>) {
        self.bindings = b.map_or(Loadable::Failed, Loadable::Ready);
        self.binding_sel = 0;
    }
    pub fn begin_routing(&mut self, zones: usize) {
        self.routing = Loadable::Loading;
        self.routing_zones = zones;
    }
    pub fn set_routing(&mut self, r: Option<RoutingInfo>) {
        self.routing = r.map_or(Loadable::Failed, Loadable::Ready);
    }

    // --- Implementaciones (pestaña 1) ---

    /// Mueve la selección de deployment (clamp, sin envolver). Devuelve si cambió.
    pub fn select_deploy(&mut self, delta: i32) -> bool {
        let Loadable::Ready(d) = &self.deployments else {
            return false;
        };
        let len = d.len();
        if len == 0 {
            return false;
        }
        let cur = self.deploy_state.selected().unwrap_or(0) as i32;
        let next = (cur + delta).clamp(0, len as i32 - 1) as usize;
        let changed = Some(next) != self.deploy_state.selected();
        self.deploy_state.select(Some(next));
        changed
    }

    pub fn selected_deploy_index(&self) -> Option<usize> {
        self.deploy_state.selected()
    }

    /// Selecciona el deployment de la fila `rel` (clic de ratón). `true` si válido.
    pub fn deploy_at(&mut self, rel: usize) -> bool {
        let Loadable::Ready(d) = &self.deployments else {
            return false;
        };
        let idx = rel + self.deploy_state.offset();
        if idx >= d.len() {
            return false;
        }
        self.deploy_state.select(Some(idx));
        true
    }

    pub fn selected_deploy(&self) -> Option<&Deployment> {
        let Loadable::Ready(d) = &self.deployments else {
            return None;
        };
        self.deploy_state.selected().and_then(|i| d.get(i))
    }

    // --- Variables (pestaña 2) ---

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
        self.log_visible.clear();
        self.log_state.select(None);
        self.log_follow = true;
        self.log_filter.set(String::new());
        self.log_errors_only = false;
    }

    /// Añade un evento al buffer (cap 1000), recomputa el filtro y reconcilia
    /// la selección (seguimiento o traducción del índice tras el drenado).
    pub fn push_event(&mut self, ev: TailEvent) {
        self.logs.push(ev);
        const CAP: usize = 1000;
        let drop = self.logs.len().saturating_sub(CAP);
        if drop > 0 {
            self.logs.drain(0..drop);
        }
        self.reconcile_logs(drop);
    }

    /// Índices de `logs` que pasan el filtro (texto sobre `summary` + solo-errores).
    fn recompute_log_visible(&mut self) {
        let needle = self.log_filter.value().trim().to_lowercase();
        let only_err = self.log_errors_only;
        self.log_visible = self
            .logs
            .iter()
            .enumerate()
            .filter(|(_, e)| {
                (!only_err || e.is_error)
                    && (needle.is_empty() || e.summary.to_lowercase().contains(&needle))
            })
            .map(|(i, _)| i)
            .collect();
    }

    /// Reconcilia la selección tras `push_event`: si seguimos, salta al último
    /// visible; si estamos en pausa, conserva el mismo evento (traduciendo el
    /// índice absoluto tras el drenado por cap).
    fn reconcile_logs(&mut self, drop: usize) {
        let prev_abs = if self.log_follow {
            None
        } else {
            self.log_state
                .selected()
                .and_then(|row| self.log_visible.get(row).copied())
                .map(|abs| abs.saturating_sub(drop))
        };
        self.recompute_log_visible();
        if self.log_follow {
            self.log_state.select(self.log_visible.len().checked_sub(1));
        } else if let Some(target) = prev_abs {
            let row = self
                .log_visible
                .iter()
                .position(|&a| a >= target)
                .or_else(|| self.log_visible.len().checked_sub(1));
            self.log_state.select(row);
        } else {
            self.clamp_log_selection();
        }
    }

    fn clamp_log_selection(&mut self) {
        match self.log_state.selected() {
            Some(s) if s < self.log_visible.len() => {}
            _ => self
                .log_state
                .select((!self.log_visible.is_empty()).then_some(0)),
        }
    }

    /// ↑↓ en la pestaña Logs: mueve selección (clamp) y pausa el seguimiento.
    /// Devuelve `true` si hay logs.
    pub fn log_scroll(&mut self, delta: i32) -> bool {
        let len = self.log_visible.len();
        if len == 0 {
            return false;
        }
        self.log_follow = false;
        let cur = self.log_state.selected().unwrap_or(0) as i32;
        let next = (cur + delta).clamp(0, len as i32 - 1) as usize;
        self.log_state.select(Some(next));
        true
    }

    /// End: vuelve al final del buffer y reactiva el seguimiento.
    pub fn log_follow_end(&mut self) {
        self.log_follow = true;
        self.log_state.select(self.log_visible.len().checked_sub(1));
    }

    pub fn selected_log_event(&self) -> Option<&TailEvent> {
        let row = self.log_state.selected()?;
        self.logs.get(*self.log_visible.get(row)?)
    }

    /// Selecciona el evento de la fila `rel` (clic de ratón); pausa el
    /// seguimiento. `true` si la fila es válida.
    pub fn log_at(&mut self, rel: usize) -> bool {
        let idx = rel + self.log_state.offset();
        if idx >= self.log_visible.len() {
            return false;
        }
        self.log_follow = false;
        self.log_state.select(Some(idx));
        true
    }

    pub fn log_filter_mut(&mut self) -> &mut TextInput {
        &mut self.log_filter
    }

    pub fn apply_log_filter(&mut self) {
        self.recompute_log_visible();
        if self.log_follow {
            self.log_state.select(self.log_visible.len().checked_sub(1));
        } else {
            self.clamp_log_selection();
        }
    }

    pub fn clear_log_filter(&mut self) {
        self.log_filter.set(String::new());
        self.apply_log_filter();
    }

    pub fn toggle_log_errors_only(&mut self) {
        self.log_errors_only = !self.log_errors_only;
        self.apply_log_filter();
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

    pub fn draw_detail(&mut self, frame: &mut Frame, area: Rect, focused: bool, filter_focused: bool) {
        // El título se calcula antes de tomar `&mut self` en los draws.
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
            3 => self.draw_logs(frame, rows[2], filter_focused),
            _ => self.draw_routing(frame, rows[2]),
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

    fn draw_deployments(&mut self, frame: &mut Frame, area: Rect) {
        match &self.deployments {
            Loadable::Idle | Loadable::Loading => {
                frame.render_widget(dim("Cargando…"), area);
            }
            Loadable::Failed => frame.render_widget(dim("No disponible"), area),
            Loadable::Ready(d) if d.is_empty() => {
                frame.render_widget(dim("Sin implementaciones"), area);
            }
            Loadable::Ready(d) => {
                let rows = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([Constraint::Min(0), Constraint::Length(1)])
                    .split(area);
                let items: Vec<ListItem> = d
                    .iter()
                    .enumerate()
                    .map(|(i, dep)| {
                        let mut spans = vec![
                            Span::raw(short_date(&dep.created_on)),
                            Span::styled(
                                format!("  {}", dep.author_email),
                                Style::default().fg(theme::FG),
                            ),
                            Span::styled(
                                format!("  ({})", dep.source),
                                Style::default().fg(theme::DIM),
                            ),
                        ];
                        if i == 0 {
                            spans.push(Span::styled(" · activo", Style::default().fg(theme::OK)));
                        }
                        ListItem::new(Line::from(spans))
                    })
                    .collect();
                let list = List::new(items)
                    .highlight_style(theme::selection())
                    .highlight_symbol("▶ ");
                frame.render_stateful_widget(list, rows[0], &mut self.deploy_state);
                frame.render_widget(
                    Paragraph::new(dim_line("Enter revertir al despliegue seleccionado")),
                    rows[1],
                );
            }
        }
    }

    fn draw_routing(&self, frame: &mut Frame, area: Rect) {
        match &self.routing {
            Loadable::Idle | Loadable::Loading => {
                frame.render_widget(
                    dim(&format!(
                        "Consultando rutas en {} zona(s)…",
                        self.routing_zones
                    )),
                    area,
                );
            }
            Loadable::Failed => frame.render_widget(dim("Rutas no disponibles"), area),
            Loadable::Ready(r) if r.routes.is_empty() && r.domains.is_empty() => {
                frame.render_widget(dim("Este worker no tiene rutas ni dominios"), area);
            }
            Loadable::Ready(r) => {
                let mut lines: Vec<Line> = Vec::new();
                if !r.routes.is_empty() {
                    lines.push(Line::from(Span::styled("Rutas de zona:", theme::title(false))));
                    for (pattern, zone) in &r.routes {
                        lines.push(Line::from(vec![
                            Span::styled("▪ ", Style::default().fg(theme::ACCENT)),
                            Span::styled(pattern.clone(), Style::default().fg(theme::FG)),
                            Span::styled(format!("  ({zone})"), Style::default().fg(theme::DIM)),
                        ]));
                    }
                }
                if !r.domains.is_empty() {
                    if !lines.is_empty() {
                        lines.push(Line::from(""));
                    }
                    lines.push(Line::from(Span::styled("Custom domains:", theme::title(false))));
                    for host in &r.domains {
                        lines.push(Line::from(vec![
                            Span::styled("▪ ", Style::default().fg(theme::ACCENT)),
                            Span::styled(host.clone(), Style::default().fg(theme::FG)),
                        ]));
                    }
                }
                frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: true }), area);
            }
        }
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

    fn draw_logs(&mut self, frame: &mut Frame, area: Rect, filter_focused: bool) {
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
            .constraints([
                Constraint::Length(1),
                Constraint::Length(1),
                Constraint::Min(0),
            ])
            .split(area);

        // Cabecera de estado.
        let (state_text, state_color) = if !self.tailing {
            ("○ detenido · 'l' para iniciar".to_string(), theme::DIM)
        } else if self.log_follow {
            ("● en vivo (siguiendo)".to_string(), theme::OK)
        } else {
            ("● en vivo · ⏸ pausado (End sigue)".to_string(), theme::WARN)
        };
        let mut header = vec![Span::styled(state_text, Style::default().fg(state_color))];
        if self.log_errors_only {
            header.push(Span::styled(" · solo errores", Style::default().fg(theme::WARN)));
        }
        let filter_val = self.log_filter.value().trim().to_string();
        if !filter_val.is_empty() {
            header.push(Span::styled(
                format!(" · /{filter_val}"),
                Style::default().fg(theme::DIM),
            ));
        }
        frame.render_widget(Paragraph::new(Line::from(header)), rows[0]);

        // Barra de filtro (o hint).
        if filter_focused || !self.log_filter.value().is_empty() {
            frame.render_widget(Paragraph::new(Line::from(self.log_filter.spans(filter_focused))), rows[1]);
        } else {
            frame.render_widget(
                Paragraph::new(Line::from(Span::styled(
                    "/ filtrar · E solo errores · Enter detalle · y copiar",
                    Style::default().fg(theme::DIM),
                ))),
                rows[1],
            );
        }

        // Lista de eventos (selección + scroll vía ListState sobre `log_visible`).
        let items: Vec<ListItem> = self
            .log_visible
            .iter()
            .filter_map(|&i| self.logs.get(i))
            .map(|e| {
                let color = if e.is_error { theme::ERROR } else { theme::FG };
                ListItem::new(Line::from(Span::styled(
                    e.summary.clone(),
                    Style::default().fg(color),
                )))
            })
            .collect();
        let list = List::new(items)
            .highlight_style(theme::selection())
            .highlight_symbol("▶ ");
        frame.render_stateful_widget(list, rows[2], &mut self.log_state);
    }
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

pub fn short_date(iso: &str) -> String {
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
