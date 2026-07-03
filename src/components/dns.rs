//! Vista del módulo DNS: lista de zonas (izq) + tabla de registros (der).

use ratatui::layout::{Constraint, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, Cell, List, ListItem, ListState, Paragraph, Row, Table, TableState, Wrap,
};
use ratatui::Frame;

use crate::model::{DnsRecord, Zone};
use crate::ui::theme;

#[derive(Default)]
pub struct DnsView {
    zones: Vec<Zone>,
    zone_state: ListState,
    records: Vec<DnsRecord>,
    record_state: TableState,
    pub loading_zones: bool,
    pub loading_records: bool,
    pub error: Option<String>,
}

impl DnsView {
    pub fn new() -> Self {
        Self::default()
    }

    // --- Zonas ---

    pub fn set_zones(&mut self, zones: Vec<Zone>) {
        self.zones = zones;
        self.loading_zones = false;
        self.error = None;
        self.zone_state
            .select(if self.zones.is_empty() { None } else { Some(0) });
        self.records.clear();
        self.record_state.select(None);
    }

    pub fn selected_zone(&self) -> Option<&Zone> {
        self.zone_state.selected().and_then(|i| self.zones.get(i))
    }

    pub fn selected_zone_id(&self) -> Option<String> {
        self.selected_zone().map(|z| z.id.clone())
    }

    /// Mueve la selección de zona; devuelve `true` si cambió.
    pub fn select_zone(&mut self, delta: i32) -> bool {
        move_selection(&mut self.zone_state, self.zones.len(), delta)
    }

    /// Selecciona una zona por fila relativa (click); `true` si cambió.
    pub fn zone_at(&mut self, rel: usize) -> bool {
        let idx = rel + self.zone_state.offset();
        if idx >= self.zones.len() {
            return false;
        }
        let changed = self.zone_state.selected() != Some(idx);
        self.zone_state.select(Some(idx));
        changed
    }

    /// Selecciona un registro por fila relativa (click).
    pub fn record_at(&mut self, rel: usize) {
        let idx = rel + self.record_state.offset();
        if idx < self.records.len() {
            self.record_state.select(Some(idx));
        }
    }

    // --- Registros ---

    pub fn set_records(&mut self, records: Vec<DnsRecord>) {
        self.records = records;
        self.loading_records = false;
        self.record_state
            .select(if self.records.is_empty() { None } else { Some(0) });
    }

    pub fn begin_loading_records(&mut self) {
        self.loading_records = true;
        self.error = None;
        self.records.clear();
        self.record_state.select(None);
    }

    pub fn selected_record(&self) -> Option<&DnsRecord> {
        self.record_state.selected().and_then(|i| self.records.get(i))
    }

    pub fn select_record(&mut self, delta: i32) {
        move_selection(&mut self.record_state, self.records.len(), delta);
    }

    // --- Render ---

    pub fn draw_zones(&mut self, frame: &mut Frame, area: Rect, focused: bool) {
        let block = Block::bordered()
            .title(" Zonas ")
            .border_style(theme::border(focused))
            .title_style(theme::title(focused));

        if self.loading_zones {
            frame.render_widget(placeholder("Cargando zonas…", block), area);
            return;
        }
        if self.zones.is_empty() {
            frame.render_widget(placeholder("Sin zonas en esta cuenta", block), area);
            return;
        }

        let items: Vec<ListItem> = self
            .zones
            .iter()
            .map(|z| ListItem::new(zone_line(z)))
            .collect();
        let list = List::new(items)
            .block(block)
            .highlight_style(theme::selection())
            .highlight_symbol("▶ ");
        frame.render_stateful_widget(list, area, &mut self.zone_state);
    }

    pub fn draw_records(&mut self, frame: &mut Frame, area: Rect, focused: bool) {
        let title = match self.selected_zone() {
            Some(z) => format!(" Registros · {} ", z.name),
            None => " Registros ".to_string(),
        };
        let block = Block::bordered()
            .title(title)
            .border_style(theme::border(focused))
            .title_style(theme::title(focused));

        if let Some(err) = &self.error {
            frame.render_widget(placeholder(&format!("✗ {err}"), block), area);
            return;
        }
        if self.loading_records {
            frame.render_widget(placeholder("Cargando registros…", block), area);
            return;
        }
        if self.records.is_empty() {
            frame.render_widget(placeholder("Sin registros", block), area);
            return;
        }

        let header = Row::new(
            ["TIPO", "NOMBRE", "CONTENIDO", "PROXY", "TTL"]
                .into_iter()
                .map(|h| Cell::from(Span::styled(h, theme::title(false)))),
        );
        let rows = self.records.iter().map(|r| {
            Row::new(vec![
                Cell::from(r.record_type.clone()),
                Cell::from(r.name.clone()),
                Cell::from(r.content.clone()),
                Cell::from(proxy_cell(r)),
                Cell::from(ttl_cell(r.ttl)),
            ])
        });
        let widths = [
            Constraint::Length(7),
            Constraint::Percentage(32),
            Constraint::Percentage(40),
            Constraint::Length(9),
            Constraint::Length(6),
        ];
        let table = Table::new(rows, widths)
            .header(header)
            .block(block)
            .row_highlight_style(theme::selection())
            .highlight_symbol("▶ ");
        frame.render_stateful_widget(table, area, &mut self.record_state);
    }
}

/// Nombre de zona + estado dim si no está `active`.
fn zone_line(z: &Zone) -> Line<'static> {
    let mut spans = vec![Span::raw(z.name.clone())];
    if !z.status.is_empty() && z.status != "active" {
        spans.push(Span::styled(
            format!("  ({})", z.status),
            Style::default().fg(theme::DIM),
        ));
    }
    Line::from(spans)
}

fn placeholder<'a>(text: &'a str, block: Block<'a>) -> Paragraph<'a> {
    Paragraph::new(text)
        .block(block)
        .style(Style::default().fg(theme::DIM))
        .wrap(Wrap { trim: true })
}

/// Celda de la columna PROXY según proxiabilidad y estado.
fn proxy_cell(r: &DnsRecord) -> Span<'static> {
    if !r.is_proxiable() {
        Span::styled("—", Style::default().fg(theme::DIM))
    } else if r.proxied == Some(true) {
        Span::styled("● on", Style::default().fg(theme::ACCENT))
    } else {
        Span::styled("○ off", Style::default().fg(theme::DIM))
    }
}

fn ttl_cell(ttl: u32) -> Line<'static> {
    Line::from(if ttl == 1 {
        "auto".to_string()
    } else {
        ttl.to_string()
    })
}

/// Mueve una selección con wrap-around dentro de `[0, len)`. `true` si cambió.
fn move_selection(state: &mut impl SelectableState, len: usize, delta: i32) -> bool {
    if len == 0 {
        return false;
    }
    let cur = state.get_selected().unwrap_or(0) as i32;
    let n = len as i32;
    let next = (((cur + delta) % n) + n) % n;
    let changed = next != cur;
    state.set_selected(Some(next as usize));
    changed
}

/// Abstracción mínima sobre `ListState`/`TableState` para reusar la navegación.
trait SelectableState {
    fn get_selected(&self) -> Option<usize>;
    fn set_selected(&mut self, i: Option<usize>);
}

impl SelectableState for ListState {
    fn get_selected(&self) -> Option<usize> {
        self.selected()
    }
    fn set_selected(&mut self, i: Option<usize>) {
        self.select(i);
    }
}

impl SelectableState for TableState {
    fn get_selected(&self) -> Option<usize> {
        self.selected()
    }
    fn set_selected(&mut self, i: Option<usize>) {
        self.select(i);
    }
}
