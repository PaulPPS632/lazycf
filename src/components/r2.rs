//! Vista del módulo R2: lista de buckets (izq) + detalle (der) con uso de
//! almacenamiento (objetos/tamaño) y dominios personalizados. Los objetos
//! (browser/subida/URLs firmadas) llegan en un incremento posterior (S3).

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, List, ListItem, ListState, Paragraph, Wrap};
use ratatui::Frame;

use crate::components::workers::Loadable;
use crate::model::{R2Bucket, R2Usage};
use crate::ui::theme;

/// Detalle + uso + dominios de un bucket (se cargan juntos).
#[derive(Debug, Clone)]
pub struct BucketInfo {
    pub detail: R2Bucket,
    pub usage: R2Usage,
    pub domains: Vec<String>,
}

#[derive(Default)]
pub struct R2View {
    buckets: Vec<R2Bucket>,
    state: ListState,
    pub loading: bool,
    pub error: Option<String>,
    info: Loadable<BucketInfo>,
    /// Nombre del bucket cuyo `info` está cargado/cargándose.
    current: Option<String>,
}

impl R2View {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_empty(&self) -> bool {
        self.buckets.is_empty()
    }

    pub fn reset(&mut self) {
        *self = Self::new();
    }

    pub fn set_buckets(&mut self, buckets: Vec<R2Bucket>) {
        self.buckets = buckets;
        self.loading = false;
        self.error = None;
        self.info = Loadable::Idle;
        self.current = None;
        self.state.select((!self.buckets.is_empty()).then_some(0));
    }

    pub fn selected(&self) -> Option<&R2Bucket> {
        self.state.selected().and_then(|i| self.buckets.get(i))
    }

    pub fn selected_name(&self) -> Option<String> {
        self.selected().map(|b| b.name.clone())
    }

    pub fn select(&mut self, delta: i32) -> bool {
        let len = self.buckets.len();
        if len == 0 {
            return false;
        }
        let cur = self.state.selected().unwrap_or(0) as i32;
        let n = len as i32;
        let next = ((((cur + delta) % n) + n) % n) as usize;
        let changed = self.state.selected() != Some(next);
        self.state.select(Some(next));
        changed
    }

    pub fn bucket_at(&mut self, rel: usize) -> bool {
        let idx = rel + self.state.offset();
        if idx >= self.buckets.len() {
            return false;
        }
        let changed = self.state.selected() != Some(idx);
        self.state.select(Some(idx));
        changed
    }

    pub fn begin_info(&mut self, bucket: String) {
        self.current = Some(bucket);
        self.info = Loadable::Loading;
    }

    pub fn set_info(&mut self, bucket: &str, info: Option<BucketInfo>) {
        if self.current.as_deref() != Some(bucket) {
            return;
        }
        self.info = info.map_or(Loadable::Failed, Loadable::Ready);
    }

    // --- Render ---

    pub fn draw_list(&mut self, frame: &mut Frame, area: Rect, focused: bool) {
        let block = Block::bordered()
            .title(" Buckets R2 ")
            .border_style(theme::border(focused))
            .title_style(theme::title(focused));
        if self.loading {
            frame.render_widget(placeholder("Cargando buckets…", block), area);
            return;
        }
        if let Some(e) = &self.error {
            frame.render_widget(placeholder(&format!("✗ {e}"), block), area);
            return;
        }
        if self.buckets.is_empty() {
            frame.render_widget(placeholder("Sin buckets en esta cuenta", block), area);
            return;
        }
        let items: Vec<ListItem> = self
            .buckets
            .iter()
            .map(|b| ListItem::new(b.name.clone()))
            .collect();
        let list = List::new(items)
            .block(block)
            .highlight_style(theme::selection())
            .highlight_symbol("▶ ");
        frame.render_stateful_widget(list, area, &mut self.state);
    }

    pub fn draw_detail(&self, frame: &mut Frame, area: Rect) {
        let title = match self.selected() {
            Some(b) => format!(" {} ", b.name),
            None => " Detalle ".to_string(),
        };
        let block = Block::bordered()
            .title(title)
            .border_style(theme::border(false))
            .title_style(theme::title(false));
        let inner = block.inner(area);
        frame.render_widget(block, area);

        if self.selected().is_none() {
            frame.render_widget(dim("Selecciona un bucket"), inner);
            return;
        }

        let lines: Vec<Line> = match &self.info {
            Loadable::Idle | Loadable::Loading => vec![dim_line("Cargando detalle…")],
            Loadable::Failed => vec![dim_line("Detalle no disponible")],
            Loadable::Ready(info) => info_lines(info),
        };
        frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: true }), inner);
    }
}

fn info_lines(info: &BucketInfo) -> Vec<Line<'static>> {
    let d = &info.detail;
    let u = &info.usage;
    let mut lines = vec![
        kv("Creado", &short_date(&d.creation_date)),
        kv("Ubicación", d.location.as_deref().unwrap_or("—")),
        kv("Clase", d.storage_class.as_deref().unwrap_or("—")),
        kv("Jurisdicción", d.jurisdiction.as_deref().unwrap_or("—")),
        Line::from(""),
        section("Almacenamiento"),
        kv("Objetos", &u.objects().to_string()),
        kv("Tamaño", &human_size(u.payload())),
        kv("Metadatos", &human_size(u.metadata())),
        kv("Subidas multipart", &u.uploads().to_string()),
        Line::from(""),
        section("Dominios personalizados"),
    ];
    if info.domains.is_empty() {
        lines.push(dim_line("  (ninguno)"));
    } else {
        for dom in &info.domains {
            lines.push(Line::from(vec![
                Span::styled("  • ", Style::default().fg(theme::ACCENT)),
                Span::styled(dom.clone(), Style::default().fg(theme::FG)),
            ]));
        }
    }
    lines.push(Line::from(""));
    lines.push(dim_line("n nuevo bucket · d borrar · r recargar"));
    lines.push(dim_line("objetos/subida/URLs firmadas → próximo incremento (S3)"));
    lines
}

/// Divide el área principal en lista (izq) + detalle (der).
pub fn split(main: Rect) -> (Rect, Rect) {
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(30), Constraint::Min(1)])
        .split(main);
    (cols[0], cols[1])
}

// --- Helpers ---

fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut size = bytes as f64;
    let mut unit = 0;
    while size >= 1024.0 && unit < UNITS.len() - 1 {
        size /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{size:.2} {}", UNITS[unit])
    }
}

fn short_date(iso: &str) -> String {
    if iso.len() >= 10 {
        iso[..10].to_string()
    } else {
        iso.to_string()
    }
}

fn kv(key: &str, value: &str) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("{key:<18}"), Style::default().fg(theme::DIM)),
        Span::styled(value.to_string(), Style::default().fg(theme::FG)),
    ])
}

fn section(title: &str) -> Line<'static> {
    Line::from(Span::styled(title.to_string(), theme::title(true)))
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
