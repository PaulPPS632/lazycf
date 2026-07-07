//! Vista del módulo D1, estilo cliente SQL:
//!   col.2 = bases (arriba) / tablas (abajo)
//!   col.3 = editor SQL (arriba) / tabla de resultados (abajo)
//! Las tablas salen de `sqlite_master`; los resultados de `POST .../raw`.

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, List, ListItem, ListState, Paragraph, Wrap};
use ratatui::Frame;

use crate::components::input::TextInput;
use crate::model::{D1Database, QueryOutcome};
use crate::ui::theme;

/// Estado del panel de resultados.
#[derive(Default)]
pub enum D1Panel {
    #[default]
    Empty,
    Loading,
    Error(String),
    Ok {
        title: String,
        outcome: QueryOutcome,
    },
}

#[derive(Default)]
pub struct D1View {
    databases: Vec<D1Database>,
    db_state: ListState,
    pub loading: bool,
    pub error: Option<String>,

    tables: Vec<String>,
    table_state: ListState,
    pub loading_tables: bool,
    tables_error: Option<String>,
    /// uuid de la base cuyas tablas están cargadas.
    current_db: Option<String>,

    /// Editor SQL (multilínea, con cursor).
    sql: TextInput,
    pub running: bool,
    result: D1Panel,
    /// Celda seleccionada en la rejilla de resultados.
    sel_row: usize,
    sel_col: usize,
    /// Desplazamiento de la rejilla (se autoajustan al renderizar).
    row_offset: usize,
    col_offset: usize,
    /// Barra WHERE: cláusula de filtro y tabla que respalda el resultado.
    where_input: TextInput,
    filter_table: Option<String>,
}

impl D1View {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_empty(&self) -> bool {
        self.databases.is_empty()
    }

    pub fn reset(&mut self) {
        *self = Self::new();
    }

    // --- Bases de datos ---

    pub fn set_databases(&mut self, dbs: Vec<D1Database>) {
        self.databases = dbs;
        self.loading = false;
        self.error = None;
        self.db_state
            .select((!self.databases.is_empty()).then_some(0));
        self.tables.clear();
        self.table_state.select(None);
        self.current_db = None;
        self.result = D1Panel::Empty;
    }

    pub fn selected_db(&self) -> Option<&D1Database> {
        self.db_state.selected().and_then(|i| self.databases.get(i))
    }

    pub fn selected_db_id(&self) -> Option<String> {
        self.selected_db().map(|d| d.uuid.clone())
    }

    pub fn selected_db_name(&self) -> Option<String> {
        self.selected_db().map(|d| d.name.clone())
    }

    pub fn select_db(&mut self, delta: i32) -> bool {
        select_in(&mut self.db_state, self.databases.len(), delta)
    }

    pub fn db_at(&mut self, rel: usize) -> bool {
        at_row(&mut self.db_state, self.databases.len(), rel)
    }

    // --- Tablas ---

    pub fn begin_tables(&mut self, db_id: String) {
        self.loading_tables = true;
        self.tables_error = None;
        self.tables.clear();
        self.table_state.select(None);
        self.current_db = Some(db_id);
    }

    pub fn set_tables(&mut self, db_id: &str, tables: Vec<String>) {
        if self.current_db.as_deref() != Some(db_id) {
            return;
        }
        self.tables = tables;
        self.loading_tables = false;
        self.tables_error = None;
        self.table_state
            .select((!self.tables.is_empty()).then_some(0));
    }

    pub fn set_tables_error(&mut self, msg: String) {
        self.loading_tables = false;
        self.tables_error = Some(msg);
    }

    pub fn selected_table(&self) -> Option<String> {
        self.table_state
            .selected()
            .and_then(|i| self.tables.get(i))
            .cloned()
    }

    pub fn select_table(&mut self, delta: i32) -> bool {
        select_in(&mut self.table_state, self.tables.len(), delta)
    }

    pub fn table_at(&mut self, rel: usize) -> bool {
        at_row(&mut self.table_state, self.tables.len(), rel)
    }

    // --- Editor SQL ---

    pub fn sql_trimmed(&self) -> String {
        self.sql.value().trim().to_string()
    }

    /// Acceso mutable al editor (para movimiento de cursor y edición desde `app`).
    pub fn editor_mut(&mut self) -> &mut TextInput {
        &mut self.sql
    }

    /// Rellena el editor (p. ej. al pulsar Enter sobre una tabla).
    pub fn set_sql(&mut self, sql: String) {
        self.sql.set(sql);
    }

    // --- Resultados ---

    pub fn begin_result(&mut self) {
        self.result = D1Panel::Loading;
        self.running = true;
        self.reset_cursor();
    }

    pub fn set_result(&mut self, title: String, outcome: QueryOutcome) {
        self.result = D1Panel::Ok { title, outcome };
        self.running = false;
        self.reset_cursor();
    }

    pub fn set_result_error(&mut self, msg: String) {
        self.result = D1Panel::Error(msg);
        self.running = false;
    }

    fn reset_cursor(&mut self) {
        self.sel_row = 0;
        self.sel_col = 0;
        self.row_offset = 0;
        self.col_offset = 0;
    }

    fn outcome(&self) -> Option<&QueryOutcome> {
        match &self.result {
            D1Panel::Ok { outcome, .. } => Some(outcome),
            _ => None,
        }
    }

    /// Mueve la celda seleccionada con clamp a las dimensiones de la tabla.
    pub fn move_cell(&mut self, dr: i32, dc: i32) {
        let Some(o) = self.outcome() else { return };
        let (rows, cols) = (o.rows.len(), o.columns.len());
        if rows == 0 || cols == 0 {
            return;
        }
        self.sel_row = (self.sel_row as i32 + dr).clamp(0, rows as i32 - 1) as usize;
        self.sel_col = (self.sel_col as i32 + dc).clamp(0, cols as i32 - 1) as usize;
    }

    /// Mueve la fila seleccionada por páginas (PgUp/PgDn).
    pub fn page_rows(&mut self, delta: i32) {
        self.move_cell(delta, 0);
    }

    /// `(columna, valor)` de la celda seleccionada.
    pub fn selected_cell_value(&self) -> Option<(String, String)> {
        let o = self.outcome()?;
        let col = o.columns.get(self.sel_col)?.clone();
        let val = o.rows.get(self.sel_row)?.get(self.sel_col)?.clone();
        Some((col, val))
    }

    /// Fila seleccionada como TSV (pegable en Excel/Sheets).
    pub fn selected_row_tsv(&self) -> Option<String> {
        let o = self.outcome()?;
        Some(o.rows.get(self.sel_row)?.join("\t"))
    }

    // --- Barra WHERE ---

    pub fn where_mut(&mut self) -> &mut TextInput {
        &mut self.where_input
    }

    pub fn where_trimmed(&self) -> String {
        self.where_input.value().trim().to_string()
    }

    pub fn filter_table(&self) -> Option<String> {
        self.filter_table.clone()
    }

    pub fn set_filter_table(&mut self, table: Option<String>) {
        self.filter_table = table;
    }

    // --- Render ---

    pub fn draw_dbs(&mut self, frame: &mut Frame, area: Rect, focused: bool) {
        let block = panel(" Bases D1 ", focused);
        if self.loading {
            frame.render_widget(dim_block("Cargando bases…", block), area);
            return;
        }
        if let Some(e) = &self.error {
            frame.render_widget(dim_block(&format!("✗ {e}"), block), area);
            return;
        }
        if self.databases.is_empty() {
            frame.render_widget(dim_block("Sin bases de datos", block), area);
            return;
        }
        let items: Vec<ListItem> = self
            .databases
            .iter()
            .map(|d| ListItem::new(d.name.clone()))
            .collect();
        let list = list_widget(items, block);
        frame.render_stateful_widget(list, area, &mut self.db_state);
    }

    pub fn draw_tables(&mut self, frame: &mut Frame, area: Rect, focused: bool) {
        let block = panel(" Tablas ", focused);
        if self.selected_db().is_none() {
            frame.render_widget(dim_block("Selecciona una base", block), area);
            return;
        }
        if self.loading_tables {
            frame.render_widget(dim_block("Cargando tablas…", block), area);
            return;
        }
        if let Some(e) = &self.tables_error {
            frame.render_widget(dim_block(&format!("✗ {e}"), block), area);
            return;
        }
        if self.tables.is_empty() {
            frame.render_widget(dim_block("Sin tablas", block), area);
            return;
        }
        let items: Vec<ListItem> = self
            .tables
            .iter()
            .map(|t| ListItem::new(t.clone()))
            .collect();
        let list = list_widget(items, block);
        frame.render_stateful_widget(list, area, &mut self.table_state);
    }

    pub fn draw_editor(&self, frame: &mut Frame, area: Rect, focused: bool) {
        let db = self.selected_db_name().unwrap_or_default();
        let title = if db.is_empty() {
            " SQL ".to_string()
        } else {
            format!(" SQL · {db} ")
        };
        let block = panel(&title, focused);
        let inner = block.inner(area);
        frame.render_widget(block, area);

        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(1), Constraint::Length(1)])
            .split(inner);

        // Cuerpo: SQL multilínea con cursor. Sin wrap para no descolocar el cursor.
        if self.sql.is_empty() && !focused {
            frame.render_widget(
                Paragraph::new(dim_line("-- escribe SQL · F5 / Ctrl+Enter ejecuta")),
                rows[0],
            );
        } else {
            frame.render_widget(Paragraph::new(self.sql.lines(focused)), rows[0]);
        }

        // Barra de estado del editor.
        let hint = if self.running {
            Span::styled("Ejecutando…", Style::default().fg(theme::ACCENT))
        } else {
            Span::styled(
                "F5 / Ctrl+Enter ejecutar · Enter salto de línea",
                Style::default().fg(theme::DIM),
            )
        };
        frame.render_widget(Paragraph::new(Line::from(hint)), rows[1]);
    }

    /// Barra WHERE: filtra los resultados de la tabla actual.
    pub fn draw_where(&self, frame: &mut Frame, area: Rect, focused: bool) {
        let block = panel(" WHERE ", focused);
        let inner = block.inner(area);
        frame.render_widget(block, area);

        let line = if self.filter_table.is_none() {
            dim_line("(aplica al seleccionar una tabla)")
        } else if self.where_input.is_empty() && !focused {
            dim_line("Escribe una cláusula WHERE para filtrar · Enter aplica")
        } else {
            Line::from(self.where_input.spans(focused))
        };
        frame.render_widget(Paragraph::new(line), inner);
    }

    pub fn draw_result(&mut self, frame: &mut Frame, area: Rect, focused: bool) {
        let title = match &self.result {
            D1Panel::Ok { title, .. } => format!(" {title} "),
            _ => " Resultado ".to_string(),
        };
        let block = panel(&title, focused);
        let inner = block.inner(area);
        frame.render_widget(block, area);

        match &self.result {
            D1Panel::Empty => {
                let lines = vec![
                    dim_line("Enter en una tabla → SELECT * LIMIT 50"),
                    dim_line("↑↓ en Tablas → ver columnas (PRAGMA)"),
                    dim_line("Editor SQL arriba → F5 para ejecutar"),
                ];
                frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: true }), inner);
            }
            D1Panel::Loading => frame.render_widget(dim("Ejecutando…"), inner),
            D1Panel::Error(e) => frame.render_widget(
                Paragraph::new(format!("✗ {e}"))
                    .style(Style::default().fg(theme::ERROR))
                    .wrap(Wrap { trim: true }),
                inner,
            ),
            // Rejilla estilo Excel + navegación de celda. Borrows disjuntos:
            // `outcome` toma `self.result`; los offsets son campos distintos.
            D1Panel::Ok { outcome, .. } => {
                let parts = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([Constraint::Min(0), Constraint::Length(1)])
                    .split(inner);
                let grid_area = parts[0];

                if outcome.columns.is_empty() {
                    frame.render_widget(
                        Paragraph::new(dim_line("(sin filas)")).wrap(Wrap { trim: true }),
                        grid_area,
                    );
                    let sum = Span::styled(outcome.summary(), Style::default().fg(theme::DIM));
                    frame.render_widget(Paragraph::new(Line::from(sum)), parts[1]);
                    return;
                }

                let w = col_widths(outcome);
                let nrows = outcome.rows.len();
                let ncols = outcome.columns.len();

                // Filas visibles (cada fila = contenido + regla; +2 cabecera/regla).
                let vis_rows = ((grid_area.height as usize).saturating_sub(2) / 2).max(1);
                if self.sel_row < self.row_offset {
                    self.row_offset = self.sel_row;
                } else if self.sel_row >= self.row_offset + vis_rows {
                    self.row_offset = self.sel_row + 1 - vis_rows;
                }
                let max_off = nrows.saturating_sub(vis_rows);
                if self.row_offset > max_off {
                    self.row_offset = max_off;
                }

                // Columnas visibles con scroll horizontal.
                if self.sel_col < self.col_offset {
                    self.col_offset = self.sel_col;
                }
                let grid_w = grid_area.width as usize;
                let mut vis_cols = fit_cols(&w, self.col_offset, grid_w);
                while !vis_cols.contains(&self.sel_col) && self.col_offset < self.sel_col {
                    self.col_offset += 1;
                    vis_cols = fit_cols(&w, self.col_offset, grid_w);
                }

                let lines = grid_lines(
                    outcome,
                    &w,
                    &vis_cols,
                    self.row_offset,
                    vis_rows,
                    self.sel_row,
                    self.sel_col,
                );
                frame.render_widget(Paragraph::new(lines), grid_area);

                // Resumen: posición + indicadores de scroll + meta.
                let end = (self.row_offset + vis_rows).min(nrows);
                let mut arrows = String::new();
                if self.row_offset > 0 {
                    arrows.push('↑');
                }
                if end < nrows {
                    arrows.push('↓');
                }
                if self.col_offset > 0 {
                    arrows.push('←');
                }
                if vis_cols.last().is_some_and(|&c| c + 1 < ncols) {
                    arrows.push('→');
                }
                let pos = format!(
                    "celda r{}/{} · c{}/{}",
                    self.sel_row + 1,
                    nrows,
                    self.sel_col + 1,
                    ncols
                );
                let sep = if arrows.is_empty() { "" } else { " " };
                let summary = format!("{pos} {arrows}{sep}· {}", outcome.summary());
                frame.render_widget(
                    Paragraph::new(Line::from(Span::styled(
                        summary,
                        Style::default().fg(theme::DIM),
                    ))),
                    parts[1],
                );
            }
        }
    }
}

/// Divide el área principal: col2 (bases/tablas) + col3 (editor/resultados).
/// Devuelve `(dbs, tables, editor, result)`.
pub fn split(main: Rect) -> (Rect, Rect, Rect, Rect) {
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(30), Constraint::Min(1)])
        .split(main);
    let left = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(45), Constraint::Min(1)])
        .split(cols[0]);
    let right = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(9), Constraint::Min(1)])
        .split(cols[1]);
    (left[0], left[1], right[0], right[1])
}

// --- Helpers ---

fn panel(title: &str, focused: bool) -> Block<'_> {
    Block::bordered()
        .title(title.to_string())
        .border_style(theme::border(focused))
        .title_style(theme::title(focused))
}

fn list_widget<'a>(items: Vec<ListItem<'a>>, block: Block<'a>) -> List<'a> {
    List::new(items)
        .block(block)
        .highlight_style(theme::selection())
        .highlight_symbol("▶ ")
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

// --- Rejilla de resultados ---

/// Ancho por columna = máx(cabecera, celdas), acotado a [3, 24].
fn col_widths(o: &QueryOutcome) -> Vec<usize> {
    let mut w: Vec<usize> = o.columns.iter().map(|c| c.chars().count()).collect();
    for row in &o.rows {
        for (i, cell) in row.iter().enumerate().take(w.len()) {
            w[i] = w[i].max(cell.chars().count());
        }
    }
    for x in &mut w {
        *x = (*x).clamp(3, 24);
    }
    w
}

/// Columnas que caben desde `start` en `width` celdas (segmento = w+2 + separador).
fn fit_cols(w: &[usize], start: usize, width: usize) -> Vec<usize> {
    let mut vis = Vec::new();
    let mut used = 0usize;
    for (i, &wi) in w.iter().enumerate().skip(start) {
        let seg = wi + 2 + usize::from(!vis.is_empty());
        if !vis.is_empty() && used + seg > width {
            break;
        }
        used += seg;
        vis.push(i);
    }
    if vis.is_empty() && start < w.len() {
        vis.push(start); // siempre al menos una
    }
    vis
}

/// Recorta a `w` caracteres (con `…`) o rellena con espacios a la derecha.
fn pad_trunc(s: &str, w: usize) -> String {
    let n = s.chars().count();
    if n > w {
        let mut out: String = s.chars().take(w.saturating_sub(1)).collect();
        out.push('…');
        out
    } else {
        format!("{s}{}", " ".repeat(w - n))
    }
}

/// Regla horizontal `─────┼─────` alineada con las columnas visibles.
fn rule_line(w: &[usize], vis_cols: &[usize], style: Style) -> Line<'static> {
    let mut s = String::new();
    for (k, &ci) in vis_cols.iter().enumerate() {
        if k > 0 {
            s.push('┼');
        }
        for _ in 0..w[ci] + 2 {
            s.push('─');
        }
    }
    Line::from(Span::styled(s, style))
}

/// Construye las líneas de la rejilla (cabecera + regla + filas con reglas).
#[allow(clippy::too_many_arguments)]
fn grid_lines(
    o: &QueryOutcome,
    w: &[usize],
    vis_cols: &[usize],
    row_off: usize,
    vis_rows: usize,
    sel_row: usize,
    sel_col: usize,
) -> Vec<Line<'static>> {
    let sep = Style::default().fg(theme::DIM);
    let mut lines: Vec<Line> = Vec::new();

    // Cabecera.
    let mut hspans: Vec<Span> = Vec::new();
    for (k, &ci) in vis_cols.iter().enumerate() {
        if k > 0 {
            hspans.push(Span::styled("│", sep));
        }
        let text = pad_trunc(&o.columns[ci], w[ci]);
        hspans.push(Span::styled(format!(" {text} "), theme::title(true)));
    }
    lines.push(Line::from(hspans));
    lines.push(rule_line(w, vis_cols, sep));

    // Filas visibles con regla entre ellas.
    let end = (row_off + vis_rows).min(o.rows.len());
    for r in row_off..end {
        let row = &o.rows[r];
        let mut spans: Vec<Span> = Vec::new();
        for (k, &ci) in vis_cols.iter().enumerate() {
            if k > 0 {
                spans.push(Span::styled("│", sep));
            }
            let cell = row.get(ci).map(String::as_str).unwrap_or("");
            let text = pad_trunc(cell, w[ci]);
            let style = if r == sel_row && ci == sel_col {
                theme::selection()
            } else {
                Style::default().fg(theme::FG)
            };
            spans.push(Span::styled(format!(" {text} "), style));
        }
        lines.push(Line::from(spans));
        lines.push(rule_line(w, vis_cols, sep));
    }
    lines
}

fn dim(text: &str) -> Paragraph<'_> {
    Paragraph::new(text)
        .style(Style::default().fg(theme::DIM))
        .wrap(Wrap { trim: true })
}

fn dim_line(text: &str) -> Line<'static> {
    Line::from(Span::styled(text.to_string(), Style::default().fg(theme::DIM)))
}

fn dim_block<'a>(text: &'a str, block: Block<'a>) -> Paragraph<'a> {
    Paragraph::new(text)
        .block(block)
        .style(Style::default().fg(theme::DIM))
        .wrap(Wrap { trim: true })
}
