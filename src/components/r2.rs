//! Vista del módulo R2, estilo explorador:
//!   col.2 = buckets (arriba) / info del bucket: peso, objetos… (abajo)
//!   col.3 = navegador de objetos (carpetas por `delimiter=/`), con subida,
//!           descarga, borrado, URLs prefirmadas y preview de imágenes.

use std::collections::HashSet;

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, List, ListItem, ListState, Paragraph, Wrap};
use ratatui::Frame;

use crate::api::r2::ObjectList;
use crate::components::input::TextInput;
use crate::model::{CustomDomain, PublicDomain, R2Bucket, R2Object, R2Usage};
use crate::ui::widgets::{dim, dim_line, human_size, metric_line, placeholder, row_at, select_wrap, short_date};
use crate::ui::{theme, Loadable};

/// Detalle + uso + dominios + CORS de un bucket (se cargan juntos).
#[derive(Debug, Clone)]
pub struct BucketInfo {
    pub detail: R2Bucket,
    pub usage: R2Usage,
    pub domains: Vec<CustomDomain>,
    pub public: PublicDomain,
    pub cors_rules: Vec<serde_json::Value>,
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

/// Modo del navegador: browse normal o resultados de búsqueda profunda.
#[derive(Debug, Default)]
pub enum BrowseMode {
    #[default]
    Normal,
    Search(SearchState),
}

/// Estado de una búsqueda profunda (todo el bucket, paginada).
#[derive(Debug)]
pub struct SearchState {
    pub term: String,
    /// `false` mientras quedan páginas por recorrer.
    pub done: bool,
    pub pages: usize,
    pub hits: usize,
    /// Se alcanzó el tope de páginas (resultados parciales).
    pub capped: bool,
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
    /// Cursor de la página siguiente del listado actual (paginación).
    next_cursor: Option<String>,
    /// Filtro de la carpeta actual (tecla `/`).
    filter: TextInput,
    /// Índices de `entries` que pasan el filtro (identidad si está vacío).
    /// La selección (`obj_state`) siempre indexa sobre esta lista.
    visible: Vec<usize>,
    /// Claves marcadas con Espacio (solo archivos) para el borrado masivo.
    marks: HashSet<String>,
    mode: BrowseMode,
    /// Generación de búsqueda: descarta respuestas de búsquedas obsoletas.
    search_gen: u64,
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
        select_wrap(&mut self.state, self.buckets.len(), delta)
    }

    pub fn bucket_at(&mut self, rel: usize) -> bool {
        row_at(&mut self.state, self.buckets.len(), rel)
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

    /// Info del bucket actual, solo si terminó de cargar con éxito.
    pub fn info(&self) -> Option<&BucketInfo> {
        match &self.info {
            Loadable::Ready(info) => Some(info),
            _ => None,
        }
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
        self.next_cursor = None;
        self.filter.set(String::new());
        self.visible.clear();
        self.marks.clear();
        self.mode = BrowseMode::Normal;
        self.search_gen += 1; // descarta búsquedas en vuelo
    }

    pub fn begin_objects(&mut self) {
        self.loading_objects = true;
        self.objects_error = None;
    }

    pub fn set_objects(&mut self, prefix: &str, mut list: ObjectList) {
        if prefix != self.prefix {
            return; // respuesta de una navegación anterior
        }
        if matches!(self.mode, BrowseMode::Search(_)) {
            return; // no pisar resultados de búsqueda con un listado tardío
        }
        self.loading_objects = false;
        self.objects_error = None;
        self.truncated = list.truncated;
        self.next_cursor = list.cursor.take();
        self.marks.clear();
        // El marcador de la propia carpeta (objeto `prefijo/`) no se lista.
        list.files.retain(|o| o.key != prefix);
        self.entries.clear();
        if !self.prefix.is_empty() {
            self.entries.push(Entry::Up);
        }
        self.entries.extend(list.folders.into_iter().map(Entry::Folder));
        self.entries.extend(list.files.into_iter().map(Entry::File));
        self.recompute_visible();
        self.obj_state
            .select((!self.visible.is_empty()).then_some(0));
    }

    /// Añade la página siguiente al listado actual (paginación por cursor).
    pub fn append_objects(&mut self, prefix: &str, mut list: ObjectList) {
        if prefix != self.prefix || matches!(self.mode, BrowseMode::Search(_)) {
            return;
        }
        self.loading_objects = false;
        self.truncated = list.truncated;
        self.next_cursor = list.cursor.take();
        // Una carpeta `delimited` puede repetirse entre páginas (prefijo que
        // cruza el corte) y conviene no duplicar claves.
        let have_folders: HashSet<&str> = self
            .entries
            .iter()
            .filter_map(|e| match e {
                Entry::Folder(p) => Some(p.as_str()),
                _ => None,
            })
            .collect();
        let new_folders: Vec<String> = list
            .folders
            .into_iter()
            .filter(|f| !have_folders.contains(f.as_str()))
            .collect();
        let have_keys: HashSet<&str> = self
            .entries
            .iter()
            .filter_map(|e| match e {
                Entry::File(o) => Some(o.key.as_str()),
                _ => None,
            })
            .collect();
        list.files
            .retain(|o| o.key != prefix && !have_keys.contains(o.key.as_str()));
        drop(have_folders);
        drop(have_keys);
        // Carpetas nuevas al final del bloque de carpetas (agrupación visual).
        let at = self
            .entries
            .iter()
            .rposition(|e| matches!(e, Entry::Up | Entry::Folder(_)))
            .map_or(0, |i| i + 1);
        for (k, f) in new_folders.into_iter().enumerate() {
            self.entries.insert(at + k, Entry::Folder(f));
        }
        self.entries.extend(list.files.into_iter().map(Entry::File));
        self.recompute_visible();
    }

    pub fn set_objects_error(&mut self, msg: String) {
        self.loading_objects = false;
        self.objects_error = Some(msg);
    }

    /// Limpia solo el flag de carga (fallo al paginar: el listado se conserva).
    /// En modo búsqueda no toca nada: ese flag lo gestiona la propia búsqueda.
    pub fn end_loading(&mut self) {
        if !self.is_searching() {
            self.loading_objects = false;
        }
    }

    pub fn selected_entry(&self) -> Option<&Entry> {
        self.obj_state
            .selected()
            .and_then(|i| self.visible.get(i))
            .and_then(|&r| self.entries.get(r))
    }

    /// `true` si ya hay un archivo listado con esa clave exacta (evita
    /// sobrescribir en silencio al renombrar). Escanea TODO el listado,
    /// no solo lo visible tras el filtro.
    pub fn key_exists(&self, key: &str) -> bool {
        self.entries
            .iter()
            .any(|e| matches!(e, Entry::File(o) if o.key == key))
    }

    /// `true` si ya existe una carpeta con ese prefijo completo.
    pub fn folder_exists(&self, full: &str) -> bool {
        self.entries
            .iter()
            .any(|e| matches!(e, Entry::Folder(p) if p == full))
    }

    /// Archivo seleccionado (si la fila actual es un archivo).
    pub fn selected_file(&self) -> Option<&R2Object> {
        match self.selected_entry() {
            Some(Entry::File(o)) => Some(o),
            _ => None,
        }
    }

    pub fn select_entry(&mut self, delta: i32) -> bool {
        select_wrap(&mut self.obj_state, self.visible.len(), delta)
    }

    pub fn entry_at(&mut self, rel: usize) -> bool {
        row_at(&mut self.obj_state, self.visible.len(), rel)
    }

    // --- Filtro de la carpeta actual ---

    /// Recalcula `visible` aplicando el filtro sobre el nombre mostrado
    /// (clave completa en modo búsqueda). Reencaja la selección si sobra.
    fn recompute_visible(&mut self) {
        let needle = self.filter.value().trim().to_lowercase();
        let searching = matches!(self.mode, BrowseMode::Search(_));
        self.visible = self
            .entries
            .iter()
            .enumerate()
            .filter(|(_, e)| {
                needle.is_empty()
                    || match e {
                        Entry::Up => true,
                        Entry::Folder(p) => folder_name(p).to_lowercase().contains(&needle),
                        Entry::File(o) if searching => o.key.to_lowercase().contains(&needle),
                        Entry::File(o) => o.filename().to_lowercase().contains(&needle),
                    }
            })
            .map(|(i, _)| i)
            .collect();
        match self.obj_state.selected() {
            Some(s) if s < self.visible.len() => {}
            _ => self
                .obj_state
                .select((!self.visible.is_empty()).then_some(0)),
        }
    }

    /// Aplica el filtro tras cada tecla (live) y selecciona la primera fila.
    pub fn apply_filter(&mut self) {
        self.recompute_visible();
        self.obj_state
            .select((!self.visible.is_empty()).then_some(0));
    }

    pub fn clear_filter(&mut self) {
        self.filter.set(String::new());
        self.apply_filter();
    }

    pub fn filter_mut(&mut self) -> &mut TextInput {
        &mut self.filter
    }

    pub fn filter_is_empty(&self) -> bool {
        self.filter.value().trim().is_empty()
    }

    // --- Paginación ---

    /// La selección está en la última fila visible.
    pub fn at_last_visible(&self) -> bool {
        !self.visible.is_empty() && self.obj_state.selected() == Some(self.visible.len() - 1)
    }

    /// Hay página siguiente que cargar (solo en browse normal).
    pub fn has_more(&self) -> bool {
        self.next_cursor.is_some() && matches!(self.mode, BrowseMode::Normal)
    }

    pub fn next_cursor_cloned(&self) -> Option<String> {
        self.next_cursor.clone()
    }

    // --- Marcas (multi-selección) ---

    /// Marca/desmarca el archivo seleccionado. `false` si la fila no es archivo.
    pub fn toggle_mark(&mut self) -> bool {
        let Some(Entry::File(o)) = self.selected_entry() else {
            return false;
        };
        let key = o.key.clone();
        if !self.marks.remove(&key) {
            self.marks.insert(key);
        }
        true
    }

    pub fn marks_len(&self) -> usize {
        self.marks.len()
    }

    /// Claves marcadas, ordenadas (confirmación y borrado deterministas).
    pub fn marked_keys(&self) -> Vec<String> {
        let mut keys: Vec<String> = self.marks.iter().cloned().collect();
        keys.sort();
        keys
    }

    pub fn clear_marks(&mut self) {
        self.marks.clear();
    }

    // --- Búsqueda profunda ---

    pub fn is_searching(&self) -> bool {
        matches!(self.mode, BrowseMode::Search(_))
    }

    /// Término de la búsqueda activa (para relanzar con `r`).
    pub fn search_term(&self) -> Option<&str> {
        match &self.mode {
            BrowseMode::Search(st) => Some(st.term.as_str()),
            BrowseMode::Normal => None,
        }
    }

    /// Entra en modo búsqueda y devuelve la generación para el guard async.
    pub fn begin_search(&mut self, term: String) -> u64 {
        self.search_gen += 1;
        self.mode = BrowseMode::Search(SearchState {
            term,
            done: false,
            pages: 0,
            hits: 0,
            capped: false,
        });
        self.entries.clear();
        self.visible.clear();
        self.marks.clear();
        self.filter.set(String::new());
        self.obj_state.select(None);
        self.next_cursor = None;
        self.loading_objects = true;
        self.objects_error = None;
        self.search_gen
    }

    /// Progreso de una búsqueda en curso. `false` si la respuesta es obsoleta.
    pub fn set_search_progress(&mut self, generation: u64, pages: usize, hits: usize) -> bool {
        if generation != self.search_gen {
            return false;
        }
        let BrowseMode::Search(st) = &mut self.mode else {
            return false;
        };
        st.pages = pages;
        st.hits = hits;
        true
    }

    /// Resultado final de la búsqueda. `false` si la respuesta es obsoleta.
    pub fn set_search_results(
        &mut self,
        generation: u64,
        files: Vec<R2Object>,
        pages: usize,
        capped: bool,
    ) -> bool {
        if generation != self.search_gen {
            return false;
        }
        let BrowseMode::Search(st) = &mut self.mode else {
            return false;
        };
        st.done = true;
        st.pages = pages;
        st.capped = capped;
        self.loading_objects = false;
        self.entries = files
            .into_iter()
            .filter(|o| !o.key.ends_with('/')) // marcadores de carpeta fuera
            .map(Entry::File)
            .collect();
        if let BrowseMode::Search(st) = &mut self.mode {
            st.hits = self.entries.len();
        }
        self.recompute_visible();
        self.obj_state
            .select((!self.visible.is_empty()).then_some(0));
        true
    }

    /// Sale del modo búsqueda; el caller recarga el browse (prefijo intacto).
    pub fn exit_search(&mut self) {
        self.search_gen += 1; // descarta tareas en vuelo
        self.mode = BrowseMode::Normal;
        self.entries.clear();
        self.visible.clear();
        self.marks.clear();
        self.filter.set(String::new());
        self.obj_state.select(None);
        self.next_cursor = None;
        self.loading_objects = false;
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
        let title = match &self.mode {
            _ if bucket.is_empty() => " Objetos ".to_string(),
            BrowseMode::Search(st) => format!(" 🔎 {bucket} · «{}» ", st.term),
            BrowseMode::Normal => format!(" 📂 {bucket}/{} ", self.prefix),
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
            let text = match &self.mode {
                BrowseMode::Search(st) => {
                    format!("Buscando… página {} · {} coincidencias", st.pages, st.hits)
                }
                BrowseMode::Normal => "Cargando objetos…".to_string(),
            };
            frame.render_widget(dim(&text), inner);
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
        if self.visible.is_empty() {
            let text = if !self.filter_is_empty() {
                format!("(sin coincidencias para «{}») · Esc limpia el filtro", self.filter.value().trim())
            } else if self.is_searching() {
                "(sin resultados) · Esc volver".to_string()
            } else {
                "(vacío) · u subir un archivo".to_string()
            };
            frame.render_widget(dim(&text), inner);
            return;
        }

        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(1), Constraint::Length(1)])
            .split(inner);

        let width = rows[0].width as usize;
        let full_key = self.is_searching();
        let items: Vec<ListItem> = self
            .visible
            .iter()
            .filter_map(|&i| self.entries.get(i))
            .map(|e| {
                let marked = matches!(e, Entry::File(o) if self.marks.contains(&o.key));
                entry_item(e, width, marked, full_key)
            })
            .collect();
        let list = List::new(items)
            .highlight_style(theme::selection())
            .highlight_symbol("▶ ");
        frame.render_stateful_widget(list, rows[0], &mut self.obj_state);

        let hint = match &self.mode {
            BrowseMode::Search(st) => {
                let mut h = format!("{} resultados · Enter ir a carpeta · Esc volver", st.hits);
                if !st.done {
                    h = format!("buscando… página {} · {h}", st.pages);
                }
                if st.capped {
                    h.push_str(" · (tope 10k alcanzado)");
                }
                h
            }
            BrowseMode::Normal => {
                let mut h =
                    String::from("Enter abrir · u subir · d descargar · p URL · x borrar · v ver");
                if self.next_cursor.is_some() {
                    h = format!("(500+ · ↓ carga más) · {h}");
                } else if self.truncated {
                    h = format!("(truncado: primeros 500) · {h}");
                }
                if !self.marks.is_empty() {
                    h = format!("{} marcados · {h}", self.marks.len());
                }
                if self.loading_objects {
                    h = format!("cargando… · {h}");
                }
                h
            }
        };
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(hint, Style::default().fg(theme::DIM)))),
            rows[1],
        );
    }

    /// Barra de filtro bajo el panel de objetos (tecla `/`).
    pub fn draw_filter(&self, frame: &mut Frame, area: Rect, focused: bool) {
        let block = Block::bordered()
            .title(" / Filtro ")
            .border_style(theme::border(focused))
            .title_style(theme::title(focused));
        let inner = block.inner(area);
        frame.render_widget(block, area);
        if self.filter.is_empty() && !focused {
            frame.render_widget(
                Paragraph::new(Span::styled(
                    "pulsa / y escribe para filtrar por nombre",
                    Style::default().fg(theme::DIM),
                )),
                inner,
            );
        } else {
            frame.render_widget(Paragraph::new(Line::from(self.filter.spans(focused))), inner);
        }
    }
}

/// Fila del navegador: icono + nombre (izq) · tamaño y fecha (der).
/// `marked` = seleccionada con Espacio; `full_key` = modo búsqueda (clave completa).
fn entry_item(entry: &Entry, width: usize, marked: bool, full_key: bool) -> ListItem<'static> {
    match entry {
        Entry::Up => ListItem::new(Line::from(Span::styled(
            "⬆ ..",
            Style::default().fg(theme::DIM),
        ))),
        Entry::Folder(prefix) => {
            let name = folder_name(prefix);
            ListItem::new(Line::from(vec![
                Span::styled("📁 ", Style::default().fg(theme::ACCENT)),
                Span::styled(format!("{name}/"), Style::default().fg(theme::FG)),
            ]))
        }
        Entry::File(o) => {
            let name = if full_key {
                o.key.clone()
            } else {
                o.filename().to_string()
            };
            let meta = format!("{:>10}  {}", human_size(o.size), short_date(&o.last_modified, 10));
            // Nombre a la izquierda, meta a la derecha (recortando el nombre).
            let avail = width.saturating_sub(meta.len() + 5).max(8);
            let shown: String = if name.chars().count() > avail {
                let cut: String = name.chars().take(avail.saturating_sub(1)).collect();
                format!("{cut}…")
            } else {
                format!("{name:<avail$}")
            };
            let (icon, icon_color) = if marked {
                ("✓ ", theme::ACCENT)
            } else if o.is_image() {
                ("🖼 ", theme::DIM)
            } else {
                ("· ", theme::DIM)
            };
            ListItem::new(Line::from(vec![
                Span::styled(icon, Style::default().fg(icon_color)),
                Span::styled(shown, Style::default().fg(theme::FG)),
                Span::styled(format!("  {meta}"), Style::default().fg(theme::DIM)),
            ]))
        }
    }
}

/// Nombre visible de una carpeta a partir de su prefijo completo.
fn folder_name(prefix: &str) -> &str {
    prefix
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or(prefix)
}

fn info_lines(info: &BucketInfo) -> Vec<Line<'static>> {
    let d = &info.detail;
    let u = &info.usage;
    let mut lines = vec![
        metric_line("Creado", &short_date(&d.creation_date, 10), 11),
        metric_line("Ubicación", d.location.as_deref().unwrap_or("—"), 11),
        metric_line("Clase", d.storage_class.as_deref().unwrap_or("—"), 11),
        Line::from(""),
        metric_line("Objetos", &u.objects().to_string(), 11),
        metric_line("Tamaño", &human_size(u.payload()), 11),
        metric_line("Metadatos", &human_size(u.metadata()), 11),
        Line::from(""),
    ];

    // Dominio público (r2.dev): existe aunque esté deshabilitado.
    let (pub_text, pub_color) = if info.public.domain.is_empty() {
        ("no disponible".to_string(), theme::DIM)
    } else if info.public.enabled {
        (format!("https://{}", info.public.domain), theme::OK)
    } else {
        (format!("{} (deshabilitado)", info.public.domain), theme::DIM)
    };
    lines.push(Line::from(vec![
        Span::styled(format!("{:<11}", "Público"), Style::default().fg(theme::DIM)),
        Span::styled(pub_text, Style::default().fg(pub_color)),
    ]));

    // CORS: solo el conteo (se edita con 'c' sobre el bucket).
    let cors_text = if info.cors_rules.is_empty() {
        "sin configurar".to_string()
    } else {
        format!("{} regla(s)", info.cors_rules.len())
    };
    lines.push(metric_line("CORS", &cors_text, 11));

    if !info.domains.is_empty() {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled("Dominios:", theme::title(false))));
        for dom in &info.domains {
            let (text, color) = if dom.enabled {
                (dom.domain.clone(), theme::FG)
            } else {
                (format!("{} (deshabilitado)", dom.domain), theme::DIM)
            };
            lines.push(Line::from(vec![
                Span::styled("• ", Style::default().fg(theme::ACCENT)),
                Span::styled(text, Style::default().fg(color)),
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

