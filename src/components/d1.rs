//! Vista del módulo D1, estilo cliente SQL:
//!   col.2 = bases (arriba) / tablas (abajo)
//!   col.3 = editor SQL (arriba) / tabla de resultados (abajo)
//! Las tablas salen de `sqlite_master`; los resultados de `POST .../raw`.

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Cell, List, ListItem, ListState, Paragraph, Row, Table, Wrap};
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
    /// Desplazamiento vertical de la tabla de resultados.
    result_scroll: usize,
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
        self.result_scroll = 0;
    }

    pub fn set_result(&mut self, title: String, outcome: QueryOutcome) {
        self.result = D1Panel::Ok { title, outcome };
        self.running = false;
        self.result_scroll = 0;
    }

    pub fn set_result_error(&mut self, msg: String) {
        self.result = D1Panel::Error(msg);
        self.running = false;
    }

    pub fn scroll_result(&mut self, delta: i32) {
        if let D1Panel::Ok { outcome, .. } = &self.result {
            let max = outcome.rows.len() as i32;
            let next = (self.result_scroll as i32 + delta).clamp(0, max.max(0));
            self.result_scroll = next as usize;
        }
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

    pub fn draw_result(&self, frame: &mut Frame, area: Rect, focused: bool) {
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
            D1Panel::Ok { outcome, .. } => {
                self.draw_outcome(frame, inner, outcome);
            }
        }
    }

    fn draw_outcome(&self, frame: &mut Frame, area: Rect, o: &QueryOutcome) {
        let rows_area = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(0), Constraint::Length(1)])
            .split(area);

        if o.columns.is_empty() {
            frame.render_widget(
                Paragraph::new(dim_line("(sin filas)")).wrap(Wrap { trim: true }),
                rows_area[0],
            );
        } else {
            // Ancho por columna = máx(cabecera, celdas), acotado.
            let ncols = o.columns.len();
            let mut w: Vec<usize> = o.columns.iter().map(|c| c.chars().count()).collect();
            for row in &o.rows {
                for (i, cell) in row.iter().enumerate().take(ncols) {
                    w[i] = w[i].max(cell.chars().count());
                }
            }
            let widths: Vec<Constraint> = w
                .iter()
                .map(|x| Constraint::Length((*x as u16).clamp(3, 40) + 1))
                .collect();

            let header = Row::new(
                o.columns
                    .iter()
                    .map(|c| Cell::from(c.clone()).style(theme::title(true))),
            )
            .style(Style::default().add_modifier(Modifier::BOLD));

            let start = self.result_scroll.min(o.rows.len());
            let body: Vec<Row> = o.rows[start..]
                .iter()
                .map(|r| Row::new(r.iter().map(|c| Cell::from(c.clone()))))
                .collect();

            let table = Table::new(body, widths)
                .header(header)
                .column_spacing(1)
                .row_highlight_style(theme::selection());
            frame.render_widget(table, rows_area[0]);
        }

        let mut summary = o.summary();
        if self.result_scroll > 0 {
            summary = format!("↓{}  {summary}", self.result_scroll);
        }
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                summary,
                Style::default().fg(theme::DIM),
            ))),
            rows_area[1],
        );
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
