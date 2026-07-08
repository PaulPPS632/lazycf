//! Vista del módulo Queues: lista de colas (izq) + detalle con pestañas
//! (Resumen · Consumers · Métricas).

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, List, ListItem, ListState, Paragraph, Sparkline, Wrap};
use ratatui::Frame;

use crate::components::r2::human_size;
use crate::components::workers::{short_date, Loadable};
use crate::model::{Queue, QueueConsumer, QueueMetrics};
use crate::ui::theme;

pub const TABS: [&str; 3] = ["Resumen", "Consumers", "Métricas"];

#[derive(Default)]
pub struct QueuesView {
    queues: Vec<Queue>,
    state: ListState,
    pub active_tab: usize,
    pub consumers: Loadable<Vec<QueueConsumer>>,
    pub metrics: Loadable<QueueMetrics>,
    /// Índice del consumer seleccionado en la pestaña Consumers.
    consumer_sel: usize,
    pub loading: bool,
    pub error: Option<String>,
}

impl QueuesView {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_empty(&self) -> bool {
        self.queues.is_empty()
    }

    pub fn reset(&mut self) {
        *self = Self::new();
    }

    /// Fija las colas conservando la selección por `queue_id` si sigue
    /// existiendo (las mutaciones pausa/purga recargan la lista).
    pub fn set_queues(&mut self, queues: Vec<Queue>) {
        let prev_id = self.selected_id();
        self.queues = queues;
        self.loading = false;
        self.error = None;
        let idx = prev_id
            .and_then(|id| self.queues.iter().position(|q| q.queue_id == id))
            .unwrap_or(0);
        self.state
            .select((!self.queues.is_empty()).then_some(idx));
    }

    /// Reinicia los datos de las pestañas (al cambiar de cola).
    pub fn reset_tabs(&mut self) {
        self.consumers = Loadable::Idle;
        self.metrics = Loadable::Idle;
        self.consumer_sel = 0;
    }

    pub fn selected(&self) -> Option<&Queue> {
        self.state.selected().and_then(|i| self.queues.get(i))
    }

    pub fn selected_id(&self) -> Option<String> {
        self.selected().map(|q| q.queue_id.clone())
    }

    pub fn selected_name(&self) -> Option<String> {
        self.selected().map(|q| q.queue_name.clone())
    }

    pub fn select(&mut self, delta: i32) -> bool {
        let len = self.queues.len();
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

    /// Selecciona la cola de la fila `rel` (clic de ratón). `true` si cambió.
    pub fn queue_at(&mut self, rel: usize) -> bool {
        let idx = rel + self.state.offset();
        if idx >= self.queues.len() {
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
    pub fn begin_consumers(&mut self) {
        self.consumers = Loadable::Loading;
    }
    pub fn set_consumers(&mut self, c: Option<Vec<QueueConsumer>>) {
        self.consumers = c.map_or(Loadable::Failed, Loadable::Ready);
        self.consumer_sel = 0;
    }
    pub fn begin_metrics(&mut self) {
        self.metrics = Loadable::Loading;
    }
    pub fn set_metrics(&mut self, m: Option<QueueMetrics>) {
        self.metrics = m.map_or(Loadable::Failed, Loadable::Ready);
    }

    // --- Consumers (pestaña 2) ---

    /// Mueve la selección de consumer (envuelve). Devuelve si cambió.
    pub fn select_consumer(&mut self, delta: i32) -> bool {
        let Loadable::Ready(cs) = &self.consumers else {
            return false;
        };
        let len = cs.len();
        if len == 0 {
            return false;
        }
        let cur = self.consumer_sel.min(len - 1) as i32;
        let n = len as i32;
        let next = ((((cur + delta) % n) + n) % n) as usize;
        let changed = next != self.consumer_sel;
        self.consumer_sel = next;
        changed
    }

    pub fn selected_consumer(&self) -> Option<&QueueConsumer> {
        let Loadable::Ready(cs) = &self.consumers else {
            return None;
        };
        cs.get(self.consumer_sel.min(cs.len().saturating_sub(1)))
    }

    /// Selecciona el consumer de la fila `rel` (clic). `true` si válida.
    pub fn consumer_at(&mut self, rel: usize) -> bool {
        let Loadable::Ready(cs) = &self.consumers else {
            return false;
        };
        if rel >= cs.len() {
            return false;
        }
        self.consumer_sel = rel;
        true
    }

    /// Consumers "efectivos" para el gating de peek/logs: los cargados
    /// (settings completos) si están; si no, los embebidos del listado.
    pub fn effective_consumers(&self) -> &[QueueConsumer] {
        if let Loadable::Ready(cs) = &self.consumers {
            return cs;
        }
        self.selected().map(|q| q.consumers.as_slice()).unwrap_or(&[])
    }

    /// Script del consumer worker para el salto a logs: el seleccionado en la
    /// pestaña Consumers si es worker; si no, el primer consumer worker.
    pub fn consumer_script(&self) -> Option<String> {
        if let Some(c) = self.selected_consumer()
            && c.is_worker()
            && let Some(s) = &c.script_name
        {
            return Some(s.clone());
        }
        self.effective_consumers()
            .iter()
            .find(|c| c.is_worker())
            .and_then(|c| c.script_name.clone())
    }

    // --- Render ---

    pub fn draw_list(&mut self, frame: &mut Frame, area: Rect, focused: bool) {
        let block = Block::bordered()
            .title(" Queues ")
            .border_style(theme::border(focused))
            .title_style(theme::title(focused));

        if self.loading {
            frame.render_widget(placeholder("Cargando colas…", block), area);
            return;
        }
        if let Some(err) = &self.error {
            frame.render_widget(placeholder(&format!("✗ {err}"), block), area);
            return;
        }
        if self.queues.is_empty() {
            frame.render_widget(
                placeholder("Sin colas · pulsa 'n' para crear una", block),
                area,
            );
            return;
        }

        let items: Vec<ListItem> = self
            .queues
            .iter()
            .map(|q| {
                let mut spans = vec![Span::styled(
                    q.queue_name.clone(),
                    Style::default().fg(theme::FG),
                )];
                if q.settings.delivery_paused {
                    spans.push(Span::styled(" ⏸", Style::default().fg(theme::WARN)));
                }
                ListItem::new(Line::from(spans))
            })
            .collect();
        let list = List::new(items)
            .block(block)
            .highlight_style(theme::selection())
            .highlight_symbol("▶ ");
        frame.render_stateful_widget(list, area, &mut self.state);
    }

    pub fn draw_detail(&mut self, frame: &mut Frame, area: Rect, focused: bool) {
        let title = match self.selected() {
            Some(q) if !q.modified_on.is_empty() => {
                format!(" {} · mod {} ", q.queue_name, short_date(&q.modified_on))
            }
            Some(q) => format!(" {} ", q.queue_name),
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
                Paragraph::new("Selecciona una cola").style(Style::default().fg(theme::DIM)),
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
            0 => self.draw_resumen(frame, rows[2]),
            1 => self.draw_consumers(frame, rows[2]),
            _ => self.draw_metrics(frame, rows[2]),
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

    fn draw_resumen(&self, frame: &mut Frame, area: Rect) {
        let Some(q) = self.selected() else {
            return;
        };
        let mut lines = vec![
            metric_line("ID", &q.queue_id),
            metric_line("Creada", &short_date(&q.created_on)),
        ];
        // Estado de la entrega.
        let (estado, color) = if q.settings.delivery_paused {
            ("⏸ pausada", theme::WARN)
        } else {
            ("● activa", theme::OK)
        };
        lines.push(Line::from(vec![
            Span::styled(format!("{:<12}", "Entrega"), Style::default().fg(theme::DIM)),
            Span::styled(estado, Style::default().fg(color)),
        ]));
        lines.push(metric_line(
            "Delay",
            &q.settings
                .delivery_delay
                .map(|d| format!("{d}s"))
                .unwrap_or_else(|| "—".into()),
        ));
        lines.push(metric_line(
            "Retención",
            &q.settings
                .message_retention_period
                .map(|s| format!("{s}s"))
                .unwrap_or_else(|| "—".into()),
        ));

        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            format!("Producers ({}):", q.producers_total_count),
            theme::title(false),
        )));
        if q.producers.is_empty() {
            lines.push(dim_line("  (ninguno)"));
        }
        for p in &q.producers {
            let label = p
                .script_name
                .clone()
                .or_else(|| p.bucket_name.clone())
                .unwrap_or_else(|| "—".into());
            lines.push(Line::from(vec![
                Span::styled("▪ ", Style::default().fg(theme::ACCENT)),
                Span::styled(format!("{:<10}", p.ptype), Style::default().fg(theme::DIM)),
                Span::styled(label, Style::default().fg(theme::FG)),
            ]));
        }

        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            format!("Consumers ({}):", q.consumers_total_count),
            theme::title(false),
        )));
        if q.consumers.is_empty() {
            lines.push(dim_line("  (ninguno)"));
        }
        for c in &q.consumers {
            lines.push(Line::from(vec![
                Span::styled("▪ ", Style::default().fg(theme::ACCENT)),
                Span::styled(format!("{:<10}", c.ctype), Style::default().fg(theme::DIM)),
                Span::styled(c.label(), Style::default().fg(theme::FG)),
            ]));
        }

        lines.push(Line::from(""));
        lines.push(dim_line(
            "s enviar · p pausa/reanuda · P purgar · m peek · l logs",
        ));
        frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: true }), area);
    }

    fn draw_consumers(&self, frame: &mut Frame, area: Rect) {
        let lines: Vec<Line> = match &self.consumers {
            Loadable::Idle | Loadable::Loading => vec![dim_line("Cargando…")],
            Loadable::Failed => vec![dim_line("No disponible")],
            Loadable::Ready(cs) if cs.is_empty() => {
                vec![dim_line("Sin consumers conectados a esta cola")]
            }
            Loadable::Ready(cs) => {
                let sel = self.consumer_sel.min(cs.len() - 1);
                let mut ls: Vec<Line> = cs
                    .iter()
                    .enumerate()
                    .map(|(i, c)| {
                        let selected = i == sel;
                        let marker = if selected { "▶ " } else { "  " };
                        let name_style = if selected {
                            Style::default()
                                .fg(theme::ACCENT)
                                .add_modifier(ratatui::style::Modifier::BOLD)
                        } else {
                            Style::default().fg(theme::ACCENT)
                        };
                        let s = &c.settings;
                        let fmt = |v: Option<u64>| {
                            v.map(|n| n.to_string()).unwrap_or_else(|| "—".into())
                        };
                        let mut extra = format!(
                            "batch {} · retries {} · delay {}s",
                            fmt(s.batch_size),
                            fmt(s.max_retries),
                            fmt(s.retry_delay)
                        );
                        if c.is_worker() {
                            extra.push_str(&format!(
                                " · conc {} · wait {}ms",
                                fmt(s.max_concurrency),
                                fmt(s.max_wait_time_ms)
                            ));
                        } else {
                            extra.push_str(&format!(" · vis {}ms", fmt(s.visibility_timeout_ms)));
                        }
                        if let Some(dlq) = &c.dead_letter_queue
                            && !dlq.is_empty()
                        {
                            extra.push_str(&format!(" · DLQ {dlq}"));
                        }
                        Line::from(vec![
                            Span::styled(
                                format!("{marker}{:<10}", c.ctype),
                                Style::default().fg(theme::DIM),
                            ),
                            Span::styled(format!("{:<24}", c.label()), name_style),
                            Span::styled(extra, Style::default().fg(theme::FG)),
                        ])
                    })
                    .collect();
                ls.push(Line::from(""));
                ls.push(dim_line("e/Enter editar · l logs (consumer worker)"));
                ls
            }
        };
        frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: true }), area);
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
                    .constraints([
                        Constraint::Length(3),
                        Constraint::Length(1),
                        Constraint::Length(4),
                        Constraint::Length(1),
                        Constraint::Min(4),
                    ])
                    .split(area);
                let head = vec![
                    metric_line("Backlog", &format!("{} mensajes", m.backlog_messages)),
                    metric_line("Tamaño", &human_size(m.backlog_bytes)),
                    Line::from(""),
                ];
                frame.render_widget(Paragraph::new(head), split[0]);
                frame.render_widget(
                    Paragraph::new(dim_line("backlog / hora (24h):")),
                    split[1],
                );
                if m.series_backlog.is_empty() {
                    frame.render_widget(dim("(sin datos)"), split[2]);
                } else {
                    let spark = Sparkline::default()
                        .data(&m.series_backlog)
                        .style(Style::default().fg(theme::ACCENT));
                    frame.render_widget(spark, split[2]);
                }
                frame.render_widget(
                    Paragraph::new(dim_line("ingeridos / hora (24h):")),
                    split[3],
                );
                if m.series_written.is_empty() {
                    frame.render_widget(dim("(sin datos)"), split[4]);
                } else {
                    let spark = Sparkline::default()
                        .data(&m.series_written)
                        .style(Style::default().fg(theme::OK));
                    frame.render_widget(spark, split[4]);
                }
            }
        }
    }
}

fn metric_line(label: &str, value: &str) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("{label:<12}"), Style::default().fg(theme::DIM)),
        Span::styled(value.to_string(), Style::default().fg(theme::FG)),
    ])
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
