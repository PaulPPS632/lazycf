//! Helpers de render y de selección compartidos por los componentes.
//!
//! Antes cada `components/*.rs` redeclaraba sus propias copias de `dim`,
//! `dim_line`, `placeholder`, `metric_line`, `tab_bar`, etc. Aquí viven una sola
//! vez; los componentes las importan de `crate::ui::widgets`.

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, ListState, Paragraph, Wrap};

use crate::components::input::TextInput;
use crate::ui::theme;

/// Párrafo tenue (texto atenuado, con ajuste de línea). Para estados vacíos
/// o de carga dentro de un área ya enmarcada.
pub fn dim(text: &str) -> Paragraph<'_> {
    Paragraph::new(text)
        .style(Style::default().fg(theme::dim()))
        .wrap(Wrap { trim: true })
}

/// Una línea tenue suelta (para listas de detalle).
pub fn dim_line(text: &str) -> Line<'static> {
    Line::from(Span::styled(
        text.to_string(),
        Style::default().fg(theme::dim()),
    ))
}

/// Párrafo tenue dentro de un bloque con borde (placeholder de un panel).
pub fn placeholder<'a>(text: &'a str, block: Block<'a>) -> Paragraph<'a> {
    Paragraph::new(text)
        .block(block)
        .style(Style::default().fg(theme::dim()))
        .wrap(Wrap { trim: true })
}

/// Fila `label` (tenue, ancho fijo) + `value` (color de primer plano).
pub fn metric_line(label: &str, value: &str, width: usize) -> Line<'static> {
    Line::from(vec![
        Span::styled(
            format!("{label:<width$}"),
            Style::default().fg(theme::dim()),
        ),
        Span::styled(value.to_string(), Style::default().fg(theme::fg())),
    ])
}

/// Fila de campo editable de un formulario: marcador de foco + etiqueta +
/// contenido del `TextInput` (con cursor si está activo).
pub fn field_row(label: &str, input: &TextInput, active: bool, width: usize) -> Line<'static> {
    let marker = if active { "▶ " } else { "  " };
    let label_style = if active {
        Style::default()
            .fg(theme::accent())
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(theme::dim())
    };
    let mut spans = vec![Span::styled(
        format!("{marker}{label:<width$}"),
        label_style,
    )];
    spans.extend(input.spans(active));
    Line::from(spans)
}

/// Barra de pestañas ` 1 Nombre · 2 Otra …` con la activa resaltada.
pub fn tab_bar(tabs: &[&str], active: usize) -> Line<'static> {
    let mut spans = Vec::new();
    for (i, t) in tabs.iter().enumerate() {
        let style = if i == active {
            Style::default()
                .fg(theme::accent())
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme::dim())
        };
        spans.push(Span::styled(format!(" {} {} ", i + 1, t), style));
        if i + 1 < tabs.len() {
            spans.push(Span::styled("·", Style::default().fg(theme::dim())));
        }
    }
    Line::from(spans)
}

/// Fila de un tema para el picker de primer arranque y la pantalla de config:
/// marcador de tema confirmado (`●`), etiqueta y 6 muestras `██` pintadas con
/// los colores del PROPIO tema listado (se ve la paleta sin activarla). `active`
/// marca el tema actualmente guardado.
pub fn theme_line(t: &theme::Theme, active: bool) -> Line<'static> {
    let marker = if active { "● " } else { "  " };
    let mut spans = vec![Span::styled(
        format!("{marker}{:<12}", t.label),
        Style::default().fg(theme::fg()),
    )];
    for c in [t.accent, t.fg, t.dim, t.error, t.ok, t.warn] {
        spans.push(Span::styled("██", Style::default().fg(c)));
    }
    Line::from(spans)
}

/// Recorta un timestamp ISO-8601 a los primeros `width` caracteres y sustituye
/// la `T` por un espacio. `width=16` → `YYYY-MM-DD HH:MM`; `width=10` → fecha.
pub fn short_date(iso: &str, width: usize) -> String {
    if iso.len() >= width {
        iso[..width].replace('T', " ")
    } else {
        iso.replace('T', " ")
    }
}

/// Mueve la selección de un `ListState` con envoltura (wrap) modular.
/// Devuelve `true` si la selección cambió.
pub fn select_wrap(state: &mut ListState, len: usize, delta: i32) -> bool {
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

/// Selecciona la fila relativa `rel` (clic de ratón), teniendo en cuenta el
/// scroll actual del `ListState`. Devuelve `true` si cayó en un elemento válido.
pub fn row_at(state: &mut ListState, len: usize, rel: usize) -> bool {
    let idx = rel + state.offset();
    if idx >= len {
        return false;
    }
    let changed = state.selected() != Some(idx);
    state.select(Some(idx));
    changed
}

/// Spans enmascarados (`•`) de un `TextInput`, con cursor de bloque en su
/// posición si `focused`. Usado por la pantalla de bienvenida y por el popup
/// de "añadir token" (antes duplicado inline en cada uno).
pub fn masked_input_spans(input: &TextInput, focused: bool) -> Vec<Span<'static>> {
    let n = input.value().chars().count();
    let bold = Style::default().fg(theme::fg()).add_modifier(Modifier::BOLD);
    if !focused {
        return vec![Span::styled("•".repeat(n), bold)];
    }
    let cur = input.cursor().min(n);
    let (at, after) = if cur < n {
        ("•".to_string(), "•".repeat(n - cur - 1))
    } else {
        (" ".to_string(), String::new())
    };
    vec![
        Span::styled("•".repeat(cur), bold),
        Span::styled(at, Style::default().add_modifier(Modifier::REVERSED)),
        Span::styled(after, bold),
    ]
}

/// Formatea un tamaño en bytes de forma legible (`B`/`KB`/`MB`/`GB`/`TB`).
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
