//! Vista del módulo R2, estilo explorador:
//!   col.2 = buckets (arriba) / info del bucket: peso, objetos… (abajo)
//!   col.3 = navegador de objetos (carpetas por `delimiter=/`), con subida,
//!           descarga, borrado, URLs prefirmadas y preview de imágenes.

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, List, ListItem, ListState, Paragraph, Wrap};
use ratatui::Frame;

use crate::api::r2::ObjectList;
use crate::components::workers::Loadable;
use crate::model::{R2Bucket, R2Object, R2Usage};
use crate::ui::theme;

/// Detalle + uso + dominios de un bucket (se cargan juntos).
#[derive(Debug, Clone)]
pub struct BucketInfo {
    pub detail: R2Bucket,
    pub usage: R2Usage,
    pub domains: Vec<String>,
}

/// Fila del navegador de objetos.
#[derive(Debug, Clone)]
pub enum Entry {
    /// Subir un nivel (`..`), visible cuando hay prefijo.
    Up,
    /// Carpeta (prefijo completo, p. ej. `empresas/1/`).
    Folder(String),
    /// Archivo.
    File(R2Object),
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

    // Navegador de objetos.
    /// Prefijo actual (`""` = raíz; siempre termina en `/` si no es raíz).
    pub prefix: String,
    entries: Vec<Entry>,
    obj_state: ListState,
    pub loading_objects: bool,
    objects_error: Option<String>,
    truncated: bool,
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

    // --- Buckets ---

    pub fn set_buckets(&mut self, buckets: Vec<R2Bucket>) {
        self.buckets = buckets;
        self.loading = false;
        self.error = None;
        self.info = Loadable::Idle;
        self.current = None;
        self.reset_browser();
        self.state.select((!self.buckets.is_empty()).then_some(0));
    }

    pub fn selected(&self) -> Option<&R2Bucket> {
        self.state.selected().and_then(|i| self.buckets.get(i))
    }

    pub fn selected_name(&self) -> Option<String> {
        self.selected().map(|b| b.name.clone())
    }

    pub fn select(&mut self, delta: i32) -> bool {
        select_in(&mut self.state, self.buckets.len(), delta)
    }

    pub fn bucket_at(&mut self, rel: usize) -> bool {
        at_row(&mut self.state, self.buckets.len(), rel)
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

    // --- Navegador de objetos ---

    /// Limpia el navegador (al cambiar de bucket/cuenta).
    pub fn reset_browser(&mut self) {
        self.prefix.clear();
        self.entries.clear();
        self.obj_state.select(None);
        self.loading_objects = false;
        self.objects_error = None;
        self.truncated = false;
    }

    pub fn begin_objects(&mut self) {
        self.loading_objects = true;
        self.objects_error = None;
    }

    pub fn set_objects(&mut self, prefix: &str, list: ObjectList) {
        if prefix != self.prefix {
            return; // respuesta de una navegación anterior
        }
        self.loading_objects = false;
        self.objects_error = None;
        self.truncated = list.truncated;
        self.entries.clear();
        if !self.prefix.is_empty() {
            self.entries.push(Entry::Up);
        }
        self.entries.extend(list.folders.into_iter().map(Entry::Folder));
        self.entries.extend(list.files.into_iter().map(Entry::File));
        self.obj_state
            .select((!self.entries.is_empty()).then_some(0));
    }

    pub fn set_objects_error(&mut self, msg: String) {
        self.loading_objects = false;
        self.objects_error = Some(msg);
    }

    pub fn selected_entry(&self) -> Option<&Entry> {
        self.obj_state.selected().and_then(|i| self.entries.get(i))
    }

    /// Archivo seleccionado (si la fila actual es un archivo).
    pub fn selected_file(&self) -> Option<&R2Object> {
        match self.selected_entry() {
            Some(Entry::File(o)) => Some(o),
            _ => None,
        }
    }

    pub fn select_entry(&mut self, delta: i32) -> bool {
        select_in(&mut self.obj_state, self.entries.len(), delta)
    }

    pub fn entry_at(&mut self, rel: usize) -> bool {
        at_row(&mut self.obj_state, self.entries.len(), rel)
    }

    /// Prefijo padre del actual (`empresas/1/` → `empresas/`).
    pub fn parent_prefix(&self) -> String {
        let trimmed = self.prefix.trim_end_matches('/');
        match trimmed.rfind('/') {
            Some(i) => trimmed[..=i].to_string(),
            None => String::new(),
        }
    }

    // --- Render ---

    pub fn draw_buckets(&mut self, frame: &mut Frame, area: Rect, focused: bool) {
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

    pub fn draw_info(&self, frame: &mut Frame, area: Rect) {
        let block = Block::bordered()
            .title(" Info ")
            .border_style(theme::border(false))
            .title_style(theme::title(false));
        let inner = block.inner(area);
        frame.render_widget(block, area);

        if self.selected().is_none() {
            frame.render_widget(dim("Selecciona un bucket"), inner);
            return;
        }
        let lines: Vec<Line> = match &self.info {
            Loadable::Idle | Loadable::Loading => vec![dim_line("Cargando info…")],
            Loadable::Failed => vec![dim_line("Info no disponible")],
            Loadable::Ready(info) => info_lines(info),
        };
        frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: true }), inner);
    }

    pub fn draw_objects(&mut self, frame: &mut Frame, area: Rect, focused: bool) {
        let bucket = self.selected_name().unwrap_or_default();
        let title = if bucket.is_empty() {
            " Objetos ".to_string()
        } else {
            format!(" 📂 {bucket}/{} ", self.prefix)
        };
        let block = Block::bordered()
            .title(title)
            .border_style(theme::border(focused))
            .title_style(theme::title(focused));
        let inner = block.inner(area);
        frame.render_widget(block, area);

        if bucket.is_empty() {
            frame.render_widget(dim("Selecciona un bucket"), inner);
            return;
        }
        if self.loading_objects && self.entries.is_empty() {
            frame.render_widget(dim("Cargando objetos…"), inner);
            return;
        }
        if let Some(e) = &self.objects_error {
            frame.render_widget(
                Paragraph::new(format!("✗ {e}"))
                    .style(Style::default().fg(theme::ERROR))
                    .wrap(Wrap { trim: true }),
                inner,
            );
            return;
        }
        if self.entries.is_empty() {
            frame.render_widget(dim("(vacío) · u subir un archivo"), inner);
            return;
        }

        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(1), Constraint::Length(1)])
            .split(inner);

        let width = rows[0].width as usize;
        let items: Vec<ListItem> = self.entries.iter().map(|e| entry_item(e, width)).collect();
        let list = List::new(items)
            .highlight_style(theme::selection())
            .highlight_symbol("▶ ");
        frame.render_stateful_widget(list, rows[0], &mut self.obj_state);

        let mut hint = String::from("Enter abrir · u subir · d descargar · p URL · x borrar · v ver");
        if self.truncated {
            hint = format!("(truncado: primeros 500) · {hint}");
        }
        if self.loading_objects {
            hint = format!("cargando… · {hint}");
        }
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(hint, Style::default().fg(theme::DIM)))),
            rows[1],
        );
    }
}

/// Fila del navegador: icono + nombre (izq) · tamaño y fecha (der).
fn entry_item(entry: &Entry, width: usize) -> ListItem<'static> {
    match entry {
        Entry::Up => ListItem::new(Line::from(Span::styled(
            "⬆ ..",
            Style::default().fg(theme::DIM),
        ))),
        Entry::Folder(prefix) => {
            let name = prefix.trim_end_matches('/').rsplit('/').next().unwrap_or(prefix);
            ListItem::new(Line::from(vec![
                Span::styled("📁 ", Style::default().fg(theme::ACCENT)),
                Span::styled(format!("{name}/"), Style::default().fg(theme::FG)),
            ]))
        }
        Entry::File(o) => {
            let name = o.filename().to_string();
            let meta = format!("{:>10}  {}", human_size(o.size), short_date(&o.last_modified));
            // Nombre a la izquierda, meta a la derecha (recortando el nombre).
            let avail = width.saturating_sub(meta.len() + 5).max(8);
            let shown: String = if name.chars().count() > avail {
                let cut: String = name.chars().take(avail.saturating_sub(1)).collect();
                format!("{cut}…")
            } else {
                format!("{name:<avail$}")
            };
            let icon = if o.is_image() { "🖼 " } else { "· " };
            ListItem::new(Line::from(vec![
                Span::styled(icon, Style::default().fg(theme::DIM)),
                Span::styled(shown, Style::default().fg(theme::FG)),
                Span::styled(format!("  {meta}"), Style::default().fg(theme::DIM)),
            ]))
        }
    }
}

fn info_lines(info: &BucketInfo) -> Vec<Line<'static>> {
    let d = &info.detail;
    let u = &info.usage;
    let mut lines = vec![
        kv("Creado", &short_date(&d.creation_date)),
        kv("Ubicación", d.location.as_deref().unwrap_or("—")),
        kv("Clase", d.storage_class.as_deref().unwrap_or("—")),
        Line::from(""),
        kv("Objetos", &u.objects().to_string()),
        kv("Tamaño", &human_size(u.payload())),
        kv("Metadatos", &human_size(u.metadata())),
    ];
    if !info.domains.is_empty() {
        lines.push(Line::from(""));
        for dom in &info.domains {
            lines.push(Line::from(vec![
                Span::styled("• ", Style::default().fg(theme::ACCENT)),
                Span::styled(dom.clone(), Style::default().fg(theme::FG)),
            ]));
        }
    }
    lines
}

/// Divide el área principal: col2 (buckets/info) + col3 (navegador).
/// Devuelve `(buckets, info, objects)`.
pub fn split(main: Rect) -> (Rect, Rect, Rect) {
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(30), Constraint::Min(1)])
        .split(main);
    let left = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(50), Constraint::Min(1)])
        .split(cols[0]);
    (left[0], left[1], cols[1])
}

// --- Imágenes: decodificar y render con medias celdas (▀ fg/bg) ---

/// Decodifica y reescala una imagen para caber en `max_cols` x `max_rows` celdas.
/// Devuelve `(ancho, alto, rgb)` listos para `image_lines`.
pub fn decode_image(
    bytes: &[u8],
    max_cols: u32,
    max_rows: u32,
) -> Result<(u32, u32, Vec<u8>), String> {
    let img = image::load_from_memory(bytes).map_err(|e| e.to_string())?;
    let img = img.to_rgb8();
    let (w, h) = (img.width().max(1), img.height().max(1));
    let (max_w, max_h) = (max_cols.max(2), max_rows.max(1) * 2);
    // Cada fila de celdas pinta 2 px de alto (media celda ▀).
    let scale = f64::min(max_w as f64 / w as f64, max_h as f64 / h as f64).min(1.0);
    let tw = ((w as f64 * scale) as u32).max(1);
    let th = ((h as f64 * scale) as u32).max(2);
    let resized = image::imageops::resize(&img, tw, th, image::imageops::FilterType::Triangle);
    Ok((tw, th, resized.into_raw()))
}

/// Convierte los píxeles RGB en líneas de `▀` (fg = píxel superior, bg = inferior).
pub fn image_lines(w: u32, h: u32, rgb: &[u8]) -> Vec<Line<'static>> {
    let px = |x: u32, y: u32| -> Color {
        let i = ((y * w + x) * 3) as usize;
        if i + 2 < rgb.len() {
            Color::Rgb(rgb[i], rgb[i + 1], rgb[i + 2])
        } else {
            Color::Black
        }
    };
    let mut lines = Vec::with_capacity(h.div_ceil(2) as usize);
    let mut y = 0;
    while y < h {
        let mut spans = Vec::with_capacity(w as usize);
        for x in 0..w {
            let top = px(x, y);
            let bottom = if y + 1 < h { px(x, y + 1) } else { Color::Black };
            spans.push(Span::styled("▀", Style::default().fg(top).bg(bottom)));
        }
        lines.push(Line::from(spans));
        y += 2;
    }
    lines
}

// --- Helpers ---

pub fn human_size(bytes: u64) -> String {
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
        Span::styled(format!("{key:<11}"), Style::default().fg(theme::DIM)),
        Span::styled(value.to_string(), Style::default().fg(theme::FG)),
    ])
}

fn select_in(state: &mut ListState, len: usize, delta: i32) -> bool {
    if len == 0 {
        return false;
    }
    let cur = state.selected().unwrap_or(0) as i32;
    let n = len as i32;
    let next = ((((cur + delta) % n) + n) % n) as usize;
    let changed = state.selected() != Some(next);
    state.select(Some(next));
    changed
}

fn at_row(state: &mut ListState, len: usize, rel: usize) -> bool {
    let idx = rel + state.offset();
    if idx >= len {
        return false;
    }
    let changed = state.selected() != Some(idx);
    state.select(Some(idx));
    changed
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
