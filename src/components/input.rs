//! Campo de texto con cursor editable (una o varias líneas). Lo usan los
//! formularios (popups) y el editor SQL de D1. Maneja movimiento de cursor
//! (←→ ↑↓ Inicio/Fin), inserción/borrado en la posición y render con cursor.

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use crate::ui::theme;

/// Buffer de texto + posición de cursor (índice de carácter, no byte).
#[derive(Default, Clone)]
pub struct TextInput {
    value: String,
    /// Cursor en `[0, num_chars]`.
    cursor: usize,
}

impl TextInput {
    pub fn new(s: impl Into<String>) -> Self {
        let value = s.into();
        let cursor = value.chars().count();
        Self { value, cursor }
    }

    pub fn value(&self) -> &str {
        &self.value
    }

    /// Copia el contenido (para construir Actions).
    pub fn take(&self) -> String {
        self.value.clone()
    }

    pub fn is_empty(&self) -> bool {
        self.value.is_empty()
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    pub fn set(&mut self, s: String) {
        self.cursor = s.chars().count();
        self.value = s;
    }

    fn char_count(&self) -> usize {
        self.value.chars().count()
    }

    /// Índice de byte del carácter `idx` (o el final).
    fn byte_at(&self, idx: usize) -> usize {
        self.value
            .char_indices()
            .nth(idx)
            .map(|(b, _)| b)
            .unwrap_or(self.value.len())
    }

    // --- Edición ---

    pub fn insert(&mut self, c: char) {
        let b = self.byte_at(self.cursor);
        self.value.insert(b, c);
        self.cursor += 1;
    }

    pub fn backspace(&mut self) {
        if self.cursor > 0 {
            let start = self.byte_at(self.cursor - 1);
            let end = self.byte_at(self.cursor);
            self.value.replace_range(start..end, "");
            self.cursor -= 1;
        }
    }

    pub fn delete(&mut self) {
        if self.cursor < self.char_count() {
            let start = self.byte_at(self.cursor);
            let end = self.byte_at(self.cursor + 1);
            self.value.replace_range(start..end, "");
        }
    }

    // --- Movimiento ---

    pub fn left(&mut self) {
        self.cursor = self.cursor.saturating_sub(1);
    }

    pub fn right(&mut self) {
        if self.cursor < self.char_count() {
            self.cursor += 1;
        }
    }

    pub fn home(&mut self) {
        self.cursor = 0;
    }

    pub fn end(&mut self) {
        self.cursor = self.char_count();
    }

    /// Línea y columna actuales del cursor (para multilínea y popups anclados).
    pub fn line_col(&self) -> (usize, usize) {
        let mut line = 0;
        let mut col = 0;
        for (i, ch) in self.value.chars().enumerate() {
            if i == self.cursor {
                break;
            }
            if ch == '\n' {
                line += 1;
                col = 0;
            } else {
                col += 1;
            }
        }
        (line, col)
    }

    fn set_line_col(&mut self, target_line: usize, col: usize) {
        let chars: Vec<char> = self.value.chars().collect();
        let mut i = 0;
        let mut line = 0;
        while i < chars.len() && line < target_line {
            if chars[i] == '\n' {
                line += 1;
            }
            i += 1;
        }
        let mut c = 0;
        while i < chars.len() && c < col && chars[i] != '\n' {
            i += 1;
            c += 1;
        }
        self.cursor = i;
    }

    /// Sube una línea manteniendo la columna (multilínea).
    pub fn up(&mut self) {
        let (line, col) = self.line_col();
        if line > 0 {
            self.set_line_col(line - 1, col);
        }
    }

    /// Baja una línea manteniendo la columna (multilínea).
    pub fn down(&mut self) {
        let (line, col) = self.line_col();
        let nlines = self.value.split('\n').count();
        if line + 1 < nlines {
            self.set_line_col(line + 1, col);
        }
    }

    /// Palabra ([A-Za-z0-9_]) inmediatamente antes del cursor.
    pub fn word_before_cursor(&self) -> String {
        let chars: Vec<char> = self.value.chars().collect();
        let end = self.cursor.min(chars.len());
        let mut start = end;
        while start > 0 && (chars[start - 1].is_alphanumeric() || chars[start - 1] == '_') {
            start -= 1;
        }
        chars[start..end].iter().collect()
    }

    /// Si justo antes de la palabra actual hay un '.', devuelve el identificador
    /// que lo precede (el alias/tabla). P. ej. con cursor tras "d." o "d.na" →
    /// `Some("d")` (para autocompletar columnas de una tabla con alias).
    pub fn alias_before_cursor(&self) -> Option<String> {
        let chars: Vec<char> = self.value.chars().collect();
        let end = self.cursor.min(chars.len());
        let mut i = end;
        while i > 0 && (chars[i - 1].is_alphanumeric() || chars[i - 1] == '_') {
            i -= 1;
        }
        if i == 0 || chars[i - 1] != '.' {
            return None;
        }
        let dot = i - 1;
        let mut start = dot;
        while start > 0 && (chars[start - 1].is_alphanumeric() || chars[start - 1] == '_') {
            start -= 1;
        }
        (start != dot).then(|| chars[start..dot].iter().collect())
    }

    /// Reemplaza la palabra antes del cursor por `replacement` (autocompletado).
    pub fn replace_word_before_cursor(&mut self, replacement: &str) {
        for _ in 0..self.word_before_cursor().chars().count() {
            self.backspace();
        }
        for c in replacement.chars() {
            self.insert(c);
        }
    }

    // --- Render ---

    /// Spans de una sola línea con cursor de bloque si `focused`.
    pub fn spans(&self, focused: bool) -> Vec<Span<'static>> {
        let base = Style::default().fg(theme::FG);
        if !focused {
            return vec![Span::styled(self.value.clone(), base)];
        }
        let chars: Vec<char> = self.value.chars().collect();
        let cur = self.cursor.min(chars.len());
        let before: String = chars[..cur].iter().collect();
        let (at, after): (String, String) = if cur < chars.len() {
            (chars[cur].to_string(), chars[cur + 1..].iter().collect())
        } else {
            (" ".to_string(), String::new())
        };
        vec![
            Span::styled(before, base),
            Span::styled(at, cursor_style()),
            Span::styled(after, base),
        ]
    }

    /// Líneas del texto (multilínea) con cursor de bloque si `focused`.
    pub fn lines(&self, focused: bool) -> Vec<Line<'static>> {
        let base = Style::default().fg(theme::FG);
        let raw: Vec<&str> = self.value.split('\n').collect();
        if !focused {
            return raw
                .iter()
                .map(|l| Line::from(Span::styled(l.to_string(), base)))
                .collect();
        }
        let (cl, cc) = self.line_col();
        raw.iter()
            .enumerate()
            .map(|(i, l)| {
                if i != cl {
                    return Line::from(Span::styled(l.to_string(), base));
                }
                let chars: Vec<char> = l.chars().collect();
                let col = cc.min(chars.len());
                let before: String = chars[..col].iter().collect();
                let (at, after): (String, String) = if col < chars.len() {
                    (chars[col].to_string(), chars[col + 1..].iter().collect())
                } else {
                    (" ".to_string(), String::new())
                };
                Line::from(vec![
                    Span::styled(before, base),
                    Span::styled(at, cursor_style()),
                    Span::styled(after, base),
                ])
            })
            .collect()
    }
}

/// Cursor de bloque (vídeo inverso).
fn cursor_style() -> Style {
    Style::default().add_modifier(Modifier::REVERSED)
}

#[cfg(test)]
mod tests {
    use super::TextInput;

    #[test]
    fn alias_tras_punto_al_final() {
        assert_eq!(
            TextInput::new("select * from documents d where d.").alias_before_cursor(),
            Some("d".to_string())
        );
    }

    #[test]
    fn alias_tras_punto_con_palabra_parcial() {
        assert_eq!(
            TextInput::new("where d.na").alias_before_cursor(),
            Some("d".to_string())
        );
    }

    #[test]
    fn sin_punto_no_hay_alias() {
        assert_eq!(TextInput::new("select * from documents").alias_before_cursor(), None);
    }
}
