//! Vista del módulo D1, estilo cliente SQL:
//!   col.2 = bases (arriba) / tablas (abajo)
//!   col.3 = editor SQL (arriba) / tabla de resultados (abajo)
//! Las tablas salen de `sqlite_master`; los resultados de `POST .../raw`.

use std::collections::HashMap;

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, List, ListItem, ListState, Paragraph, Wrap};
use ratatui::Frame;

use crate::components::input::TextInput;
use crate::model::{D1Database, QueryOutcome};
use crate::ui::theme;
use crate::ui::widgets::{dim, dim_line, placeholder, row_at, select_wrap};

/// Keywords SQL ofrecidas por el autocompletado (en MAYÚSCULA).
const SQL_KEYWORDS: &[&str] = &[
    "SELECT", "FROM", "WHERE", "AND", "OR", "NOT", "NULL", "IS", "IN", "LIKE", "BETWEEN",
    "EXISTS", "INSERT INTO", "VALUES", "UPDATE", "SET", "DELETE FROM", "CREATE TABLE",
    "DROP TABLE", "ALTER TABLE", "JOIN", "LEFT JOIN", "INNER JOIN", "CROSS JOIN", "ON", "AS",
    "GROUP BY", "ORDER BY", "HAVING", "LIMIT", "OFFSET", "DISTINCT", "ASC", "DESC", "CASE",
    "WHEN", "THEN", "ELSE", "END", "UNION", "PRAGMA", "COUNT(", "SUM(", "AVG(", "MIN(", "MAX(",
];

/// Tipo de sugerencia (define color y prioridad contextual).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum SugKind {
    Keyword,
    Table,
    Column,
}

pub struct Suggestion {
    pub text: String,
    pub kind: SugKind,
}

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
    /// Esquema (tabla → columnas) para el autocompletado.
    schema: HashMap<String, Vec<String>>,
    /// Sugerencias visibles y su selección.
    suggestions: Vec<Suggestion>,
    sug_idx: usize,
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
        select_wrap(&mut self.db_state, self.databases.len(), delta)
    }

    pub fn db_at(&mut self, rel: usize) -> bool {
        row_at(&mut self.db_state, self.databases.len(), rel)
    }

    // --- Tablas ---

    pub fn begin_tables(&mut self, db_id: String) {
        self.loading_tables = true;
        self.tables_error = None;
        self.tables.clear();
        self.table_state.select(None);
        self.current_db = Some(db_id);
        self.schema.clear();
        self.close_suggestions();
    }

    pub fn set_tables(
        &mut self,
        db_id: &str,
        tables: Vec<String>,
        schema: HashMap<String, Vec<String>>,
    ) {
        if self.current_db.as_deref() != Some(db_id) {
            return;
        }
        self.tables = tables;
        self.schema = schema;
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
        select_wrap(&mut self.table_state, self.tables.len(), delta)
    }

    pub fn table_at(&mut self, rel: usize) -> bool {
        row_at(&mut self.table_state, self.tables.len(), rel)
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
        self.close_suggestions();
    }

    // --- Autocompletado ---

    pub fn suggestions_open(&self) -> bool {
        !self.suggestions.is_empty()
    }

    pub fn close_suggestions(&mut self) {
        self.suggestions.clear();
        self.sug_idx = 0;
    }

    pub fn sug_move(&mut self, delta: i32) {
        let n = self.suggestions.len() as i32;
        if n == 0 {
            return;
        }
        self.sug_idx = ((((self.sug_idx as i32 + delta) % n) + n) % n) as usize;
    }

    /// Inserta la sugerencia seleccionada reemplazando la palabra actual.
    pub fn accept_suggestion(&mut self) {
        if let Some(s) = self.suggestions.get(self.sug_idx) {
            let text = s.text.clone();
            self.sql.replace_word_before_cursor(&text);
            self.close_suggestions();
        }
    }

    /// Recalcula las sugerencias para la palabra bajo el cursor.
    /// `forced` (Ctrl+Espacio) abre el popup aunque la palabra esté vacía.
    pub fn update_suggestions(&mut self, forced: bool) {
        let word = self.sql.word_before_cursor();

        // "alias." → solo columnas de la tabla/subquery referenciada (vía
        // FROM/JOIN ... alias, o directamente "tabla." sin alias). Señal
        // inequívoca: se evalúa antes del corte por palabra vacía (no
        // requiere Ctrl+Espacio).
        if let Some(alias) = self.sql.alias_before_cursor() {
            let tokens = tokenize(self.sql.value());
            let alias_cols = extract_alias_columns(&tokens, &self.schema);
            let cols = alias_cols
                .get(&alias.to_lowercase())
                .cloned()
                .or_else(|| schema_columns(&self.schema, &alias).cloned());
            let wl = word.to_lowercase();
            self.suggestions = cols
                .map(|cols| {
                    let mut v: Vec<&String> = cols
                        .iter()
                        .filter(|c| wl.is_empty() || c.to_lowercase().starts_with(&wl))
                        .collect();
                    v.sort();
                    v.into_iter()
                        .take(8)
                        .map(|c| Suggestion {
                            text: c.clone(),
                            kind: SugKind::Column,
                        })
                        .collect()
                })
                .unwrap_or_default();
            self.sug_idx = 0;
            return;
        }

        if word.is_empty() && !forced {
            self.close_suggestions();
            return;
        }

        // Contexto: texto antes de la palabra actual.
        let chars: Vec<char> = self.sql.value().chars().collect();
        let end = self.sql.cursor().min(chars.len());
        let before: String = chars[..end - word.chars().count()].iter().collect();
        let ctx = context_kind(&before);

        // Columnas: de las tablas mencionadas en el SQL; si ninguna, de todas.
        let text_lower = self.sql.value().to_lowercase();
        let mentioned: Vec<&String> = self
            .schema
            .keys()
            .filter(|t| text_lower.contains(&t.to_lowercase()))
            .collect();
        let mut columns: Vec<&String> = if mentioned.is_empty() {
            self.schema.values().flatten().collect()
        } else {
            mentioned
                .iter()
                .filter_map(|t| self.schema.get(*t))
                .flatten()
                .collect()
        };
        columns.sort();
        columns.dedup();

        let wl = word.to_lowercase();
        let matches = |s: &str| wl.is_empty() || s.to_lowercase().starts_with(&wl);
        let mut out: Vec<Suggestion> = Vec::new();
        if ctx != Ctx::Tables {
            out.extend(columns.iter().filter(|c| matches(c)).map(|c| Suggestion {
                text: (*c).clone(),
                kind: SugKind::Column,
            }));
        }
        out.extend(self.tables.iter().filter(|t| matches(t)).map(|t| Suggestion {
            text: t.clone(),
            kind: SugKind::Table,
        }));
        if ctx != Ctx::Tables {
            out.extend(
                SQL_KEYWORDS
                    .iter()
                    .filter(|k| matches(k))
                    .map(|k| Suggestion {
                        text: (*k).to_string(),
                        kind: SugKind::Keyword,
                    }),
            );
        }

        // Prioridad por contexto, alfabético dentro de cada tipo.
        let rank = |k: SugKind| match (ctx, k) {
            (Ctx::Tables, SugKind::Table) => 0,
            (Ctx::Columns, SugKind::Column) => 0,
            (Ctx::Columns, SugKind::Keyword) => 1,
            (Ctx::Columns, SugKind::Table) => 2,
            (Ctx::Any, SugKind::Keyword) => 0,
            (Ctx::Any, SugKind::Table) => 1,
            (Ctx::Any, SugKind::Column) => 2,
            _ => 3,
        };
        out.sort_by(|a, b| {
            rank(a.kind)
                .cmp(&rank(b.kind))
                .then_with(|| a.text.to_lowercase().cmp(&b.text.to_lowercase()))
        });
        // No molestar si lo escrito ya es la única sugerencia exacta.
        if out.len() == 1 && out[0].text.eq_ignore_ascii_case(&word) {
            out.clear();
        }
        out.truncate(8);
        self.suggestions = out;
        self.sug_idx = 0;
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
            frame.render_widget(placeholder("Cargando bases…", block), area);
            return;
        }
        if let Some(e) = &self.error {
            frame.render_widget(placeholder(&format!("✗ {e}"), block), area);
            return;
        }
        if self.databases.is_empty() {
            frame.render_widget(placeholder("Sin bases de datos", block), area);
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
            frame.render_widget(placeholder("Selecciona una base", block), area);
            return;
        }
        if self.loading_tables {
            frame.render_widget(placeholder("Cargando tablas…", block), area);
            return;
        }
        if let Some(e) = &self.tables_error {
            frame.render_widget(placeholder(&format!("✗ {e}"), block), area);
            return;
        }
        if self.tables.is_empty() {
            frame.render_widget(placeholder("Sin tablas", block), area);
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
                "F5 / Ctrl+Enter ejecutar · Ctrl+Espacio sugerir",
                Style::default().fg(theme::DIM),
            )
        };
        frame.render_widget(Paragraph::new(Line::from(hint)), rows[1]);

        // Popup de autocompletado anclado bajo el cursor (estilo IDE; puede
        // solapar el panel de abajo).
        if focused && self.suggestions_open() {
            self.draw_suggestions(frame, rows[0]);
        }
    }

    /// Overlay de sugerencias en coordenadas absolutas del frame.
    fn draw_suggestions(&self, frame: &mut Frame, text_area: Rect) {
        let screen = frame.area();
        let (line, col) = self.sql.line_col();
        let word_len = self.sql.word_before_cursor().chars().count();

        let width = self
            .suggestions
            .iter()
            .map(|s| s.text.chars().count())
            .max()
            .unwrap_or(0) as u16
            + 2;
        let height = self.suggestions.len() as u16;

        let anchor_x = text_area.x + (col.saturating_sub(word_len)) as u16;
        let x = anchor_x.min(screen.right().saturating_sub(width.min(screen.width)));
        let below = text_area.y + line as u16 + 1;
        let y = if below + height <= screen.bottom() {
            below
        } else {
            // No cabe abajo: encima de la línea del cursor.
            (text_area.y + line as u16).saturating_sub(height)
        };
        let rect = Rect::new(
            x,
            y,
            width.min(screen.width),
            height.min(screen.height),
        )
        .intersection(screen);
        if rect.is_empty() {
            return;
        }

        let lines: Vec<Line> = self
            .suggestions
            .iter()
            .enumerate()
            .map(|(i, s)| {
                let color = match s.kind {
                    SugKind::Keyword => theme::ACCENT,
                    SugKind::Table => theme::OK,
                    SugKind::Column => theme::FG,
                };
                let style = if i == self.sug_idx {
                    theme::selection()
                } else {
                    Style::default().fg(color)
                };
                let pad = (rect.width as usize).saturating_sub(s.text.chars().count() + 1);
                Line::from(Span::styled(
                    format!(" {}{}", s.text, " ".repeat(pad)),
                    style,
                ))
            })
            .collect();
        frame.render_widget(Clear, rect);
        frame.render_widget(Paragraph::new(lines), rect);
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

// --- Autocompletado: contexto ---

/// Qué priorizar según lo que precede a la palabra actual.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Ctx {
    /// Tras FROM/JOIN/INTO/UPDATE/TABLE: solo tablas.
    Tables,
    /// Tras SELECT/WHERE/ON/BY/SET… o coma/paréntesis: columnas primero.
    Columns,
    /// Sin pista: keywords primero.
    Any,
}

/// Separa en palabras (`[A-Za-z0-9_]+`) y símbolos sueltos (cualquier otro
/// carácter no-espacio, cada uno su propio token). Distingue `FROM a, b`
/// (token `,` entre `a` y `b`) de `FROM documents d` (alias real).
fn tokenize(sql: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut cur = String::new();
    for c in sql.chars() {
        if c.is_alphanumeric() || c == '_' {
            cur.push(c);
            continue;
        }
        if !cur.is_empty() {
            tokens.push(std::mem::take(&mut cur));
        }
        if !c.is_whitespace() {
            tokens.push(c.to_string());
        }
    }
    if !cur.is_empty() {
        tokens.push(cur);
    }
    tokens
}

/// Palabras que, si aparecen justo tras una tabla, indican que NO hay alias
/// (la tabla se usa sola y lo que sigue es la siguiente cláusula).
const RESERVED_AFTER_TABLE: &[&str] = &[
    "WHERE", "GROUP", "ORDER", "HAVING", "LIMIT", "OFFSET", "JOIN", "LEFT", "RIGHT", "INNER",
    "CROSS", "FULL", "OUTER", "ON", "UNION", "SET", "AND", "OR", "VALUES", "RETURNING", "AS",
];

fn is_word(t: &str) -> bool {
    t.chars().next().is_some_and(|c| c.is_alphanumeric() || c == '_')
}

/// Columnas de `table` en `schema`, comparando el nombre case-insensitive.
fn schema_columns<'a>(schema: &'a HashMap<String, Vec<String>>, table: &str) -> Option<&'a Vec<String>> {
    schema.iter().find(|(k, _)| k.eq_ignore_ascii_case(table)).map(|(_, v)| v)
}

/// Índice del `)` que cierra el `(` en `open` (tokens ya balanceados).
fn matching_paren(tokens: &[String], open: usize) -> Option<usize> {
    let mut depth = 0i32;
    for (i, t) in tokens.iter().enumerate().skip(open) {
        match t.as_str() {
            "(" => depth += 1,
            ")" => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
    }
    None
}

/// Nombre de columna/alias de un segmento de lista SELECT (best-effort):
/// alias explícito tras `AS`; si no, dos palabras seguidas al final
/// (`precio total` → `total`); si no, el último identificador del segmento
/// (`t.nombre` → `nombre`; `COUNT(id)` → `id`, imperfecto pero inofensivo).
fn column_alias(seg: &[&String]) -> Option<String> {
    if seg.is_empty() {
        return None;
    }
    let mut depth = 0i32;
    for (i, t) in seg.iter().enumerate() {
        match t.as_str() {
            "(" => depth += 1,
            ")" => depth -= 1,
            _ if depth == 0 && t.eq_ignore_ascii_case("AS") => {
                return seg.get(i + 1).map(|s| (*s).clone());
            }
            _ => {}
        }
    }
    if seg.len() >= 2 && is_word(seg[seg.len() - 1]) && is_word(seg[seg.len() - 2]) {
        return Some(seg[seg.len() - 1].clone());
    }
    seg.iter().rev().find(|t| is_word(t)).map(|s| (*s).clone())
}

/// Tablas nombradas tras FROM/JOIN en `tokens` (ignora subqueries anidadas:
/// si tras FROM/JOIN viene `(`, no es un nombre de tabla y se salta).
fn referenced_tables(tokens: &[String]) -> Vec<String> {
    let mut tables = Vec::new();
    for (i, t) in tokens.iter().enumerate() {
        let kw = t.to_uppercase();
        if (kw == "FROM" || kw == "JOIN") && tokens.get(i + 1).is_some_and(|n| is_word(n)) {
            tables.push(tokens[i + 1].clone());
        }
    }
    tables
}

/// Unión (deduplicada, en orden) de las columnas de las tablas referenciadas.
fn referenced_columns(tokens: &[String], schema: &HashMap<String, Vec<String>>) -> Vec<String> {
    let mut cols = Vec::new();
    for t in referenced_tables(tokens) {
        if let Some(found) = schema_columns(schema, &t) {
            for c in found {
                if !cols.contains(c) {
                    cols.push(c.clone());
                }
            }
        }
    }
    cols
}

/// Añade las columnas de un segmento de lista SELECT (`t.*` expande a todas
/// las columnas de `t` — tabla real o alias local del FROM interno—; el
/// resto aporta una sola columna).
fn push_segment_columns(
    seg: &[&String],
    schema: &HashMap<String, Vec<String>>,
    local_aliases: &HashMap<String, Vec<String>>,
    out: &mut Vec<String>,
) {
    if seg.len() >= 3 && seg[seg.len() - 1] == "*" && seg[seg.len() - 2] == "." {
        let name = seg[seg.len() - 3];
        if let Some(found) = schema_columns(schema, name) {
            out.extend(found.iter().cloned());
        } else if let Some(found) = local_aliases.get(&name.to_lowercase()) {
            out.extend(found.iter().cloned());
        }
        return;
    }
    if let Some(name) = column_alias(seg) {
        out.push(name);
    }
}

/// Columnas de salida de un SELECT interno (el contenido entre los paréntesis
/// de una subquery en FROM/JOIN). Best-effort: `SELECT *` expande con las
/// tablas del FROM interno; el resto, un nombre por columna de la lista.
fn subquery_columns(tokens: &[String], schema: &HashMap<String, Vec<String>>) -> Vec<String> {
    let Some(select_at) = tokens.iter().position(|t| t.eq_ignore_ascii_case("SELECT")) else {
        return Vec::new();
    };
    let mut depth = 0i32;
    let mut from_at = None;
    for (i, t) in tokens.iter().enumerate().skip(select_at + 1) {
        match t.as_str() {
            "(" => depth += 1,
            ")" => depth -= 1,
            _ if depth == 0 && t.eq_ignore_ascii_case("FROM") => {
                from_at = Some(i);
                break;
            }
            _ => {}
        }
    }
    let list = &tokens[select_at + 1..from_at.unwrap_or(tokens.len())];

    if list.len() == 1 && list[0] == "*" {
        return from_at
            .map(|f| referenced_columns(&tokens[f..], schema))
            .unwrap_or_default();
    }

    // Alias locales del FROM interno (p. ej. "documents d"): necesarios para
    // resolver "d.*" en la lista de columnas, donde `d` no es un nombre real.
    let local_aliases = from_at
        .map(|f| extract_alias_columns(&tokens[f..], schema))
        .unwrap_or_default();

    let mut cols = Vec::new();
    let mut depth = 0i32;
    let mut seg: Vec<&String> = Vec::new();
    for t in list {
        match t.as_str() {
            "(" => {
                depth += 1;
                seg.push(t);
            }
            ")" => {
                depth -= 1;
                seg.push(t);
            }
            "," if depth == 0 => {
                push_segment_columns(&seg, schema, &local_aliases, &mut cols);
                seg.clear();
            }
            _ => seg.push(t),
        }
    }
    push_segment_columns(&seg, schema, &local_aliases, &mut cols);
    cols
}

/// Alias (de tabla real o de subquery) → columnas, escaneando `FROM`/`JOIN`
/// seguidos de `tabla [AS] alias` o `( SELECT … ) [AS] alias`. Best-effort
/// (sin parser SQL real): ignora `FROM a, b` (el token tras la tabla es `,`,
/// no una palabra) y cualquier cláusula que venga justo después de la tabla
/// (WHERE, GROUP BY, otro JOIN…).
fn extract_alias_columns(
    tokens: &[String],
    schema: &HashMap<String, Vec<String>>,
) -> HashMap<String, Vec<String>> {
    let mut out = HashMap::new();
    let mut i = 0;
    while i < tokens.len() {
        let kw = tokens[i].to_uppercase();
        if kw != "FROM" && kw != "JOIN" {
            i += 1;
            continue;
        }

        if tokens.get(i + 1).map(String::as_str) == Some("(") {
            let Some(close) = matching_paren(tokens, i + 1) else {
                i += 1;
                continue;
            };
            let cols = subquery_columns(&tokens[i + 2..close], schema);
            let mut j = close + 1;
            if tokens.get(j).is_some_and(|t| t.eq_ignore_ascii_case("AS")) {
                j += 1;
            }
            if let Some(alias) = tokens.get(j)
                && is_word(alias)
                && !RESERVED_AFTER_TABLE.contains(&alias.to_uppercase().as_str())
            {
                out.insert(alias.to_lowercase(), cols);
                i = j + 1;
                continue;
            }
            i = close + 1;
            continue;
        }

        if tokens.get(i + 1).is_some_and(|t| is_word(t)) {
            let table = tokens[i + 1].clone();
            let mut j = i + 2;
            if tokens.get(j).is_some_and(|t| t.eq_ignore_ascii_case("AS")) {
                j += 1;
            }
            if let Some(alias) = tokens.get(j)
                && is_word(alias)
                && !RESERVED_AFTER_TABLE.contains(&alias.to_uppercase().as_str())
            {
                if let Some(cols) = schema_columns(schema, &table) {
                    out.insert(alias.to_lowercase(), cols.clone());
                }
                i = j + 1;
                continue;
            }
            i += 2;
            continue;
        }
        i += 1;
    }
    out
}

fn context_kind(before: &str) -> Ctx {
    let trimmed = before.trim_end();
    if trimmed.ends_with(',') || trimmed.ends_with('(') {
        return Ctx::Columns;
    }
    let last = trimmed
        .rsplit(|c: char| !c.is_alphanumeric() && c != '_')
        .find(|w| !w.is_empty())
        .unwrap_or("")
        .to_uppercase();
    match last.as_str() {
        "FROM" | "JOIN" | "INTO" | "UPDATE" | "TABLE" => Ctx::Tables,
        "SELECT" | "WHERE" | "AND" | "OR" | "ON" | "BY" | "SET" | "HAVING" | "DISTINCT" => {
            Ctx::Columns
        }
        _ => Ctx::Any,
    }
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

#[cfg(test)]
mod tests {
    use super::{extract_alias_columns, tokenize};
    use std::collections::HashMap;

    fn schema() -> HashMap<String, Vec<String>> {
        HashMap::from([("documents".to_string(), vec!["id".to_string(), "name".to_string()])])
    }

    fn aliases(sql: &str) -> HashMap<String, Vec<String>> {
        extract_alias_columns(&tokenize(sql), &schema())
    }

    #[test]
    fn alias_con_as() {
        let a = aliases("select * from documents as d where d.x = 1");
        assert_eq!(a.get("d"), Some(&vec!["id".to_string(), "name".to_string()]));
    }

    #[test]
    fn alias_sin_as() {
        let a = aliases("select * from documents d where d.x = 1");
        assert_eq!(a.get("d"), Some(&vec!["id".to_string(), "name".to_string()]));
    }

    #[test]
    fn coma_no_genera_alias_falso() {
        let a = aliases("select a, b from documents");
        assert!(a.is_empty());
    }

    #[test]
    fn tabla_sin_alias() {
        let a = aliases("select * from documents where x = 1");
        assert!(a.is_empty());
    }

    #[test]
    fn join_con_alias() {
        let a = aliases("from a inner join documents d on a.id = d.a_id");
        assert_eq!(a.get("d"), Some(&vec!["id".to_string(), "name".to_string()]));
    }

    #[test]
    fn subquery_select_star() {
        let a = aliases("select * from (select * from documents) x where x.");
        assert_eq!(a.get("x"), Some(&vec!["id".to_string(), "name".to_string()]));
    }

    #[test]
    fn subquery_lista_explicita_con_alias() {
        let a = aliases("select * from (select id, name as n from documents) x");
        assert_eq!(a.get("x"), Some(&vec!["id".to_string(), "n".to_string()]));
    }

    #[test]
    fn subquery_table_star() {
        let a = aliases("select * from (select d.* from documents d) x");
        assert_eq!(a.get("x"), Some(&vec!["id".to_string(), "name".to_string()]));
    }
}
