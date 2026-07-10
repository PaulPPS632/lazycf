//! Pantalla de bienvenida (onboarding): login (OAuth/token) y, en el primer
//! arranque, selección de tema. Sustituye a los popups `ThemePicker` y al
//! `Token` inicial: se dibuja a pantalla completa mientras `Screen::Auth` está
//! activo. El estado vive en `App::welcome`; las teclas/clics las procesa
//! `App::welcome_key`/`welcome_click` (patrón de `ConfigView`).

use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, List, ListItem, ListState, Paragraph, Wrap};

use crate::components::input::TextInput;
use crate::ui::widgets::{self, theme_line};
use crate::ui::{layout, theme};

/// Paso del onboarding. `Theme` solo se alcanza en el primer arranque.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WelcomeStep {
    Auth,
    Theme,
}

/// Foco dentro del paso de autenticación. Ciclo: Url → Token → Continue → Back.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WelcomeFocus {
    Url,
    Token,
    Continue,
    Back,
}

impl WelcomeFocus {
    fn next(self) -> Self {
        match self {
            WelcomeFocus::Url => WelcomeFocus::Token,
            WelcomeFocus::Token => WelcomeFocus::Continue,
            WelcomeFocus::Continue => WelcomeFocus::Back,
            WelcomeFocus::Back => WelcomeFocus::Url,
        }
    }
    fn prev(self) -> Self {
        match self {
            WelcomeFocus::Url => WelcomeFocus::Back,
            WelcomeFocus::Token => WelcomeFocus::Url,
            WelcomeFocus::Continue => WelcomeFocus::Token,
            WelcomeFocus::Back => WelcomeFocus::Continue,
        }
    }
}

/// Estado de la pantalla de bienvenida completa.
pub struct WelcomeView {
    pub step: WelcomeStep,
    pub focus: WelcomeFocus,
    pub token_input: TextInput,
    pub error: Option<String>,
    /// Verificando un token recién enviado o el resultado de OAuth.
    pub verifying: bool,
    /// Verificando credenciales YA guardadas al arrancar: si todas fallan,
    /// recién ahí se auto-inicia OAuth (no se relanza por un token manual malo).
    pub verifying_stored: bool,
    pub oauth_in_progress: bool,
    pub oauth_url: Option<String>,
    /// Feedback puntual "copiada" tras Enter/click sobre la URL.
    pub copied: bool,

    // Paso de tema.
    pub theme_state: ListState,
    pub theme_on_continue: bool,

    // Rects del último frame (hit-testing de mouse).
    pub rect_url: Option<Rect>,
    pub rect_token: Option<Rect>,
    pub rect_continue: Option<Rect>,
    pub rect_back: Option<Rect>,
    pub rect_themes: Option<Rect>,
}

impl WelcomeView {
    pub fn new() -> Self {
        let mut theme_state = ListState::default();
        theme_state.select(Some(theme::current_index()));
        Self {
            step: WelcomeStep::Auth,
            focus: WelcomeFocus::Url,
            token_input: TextInput::default(),
            error: None,
            verifying: false,
            verifying_stored: false,
            oauth_in_progress: false,
            oauth_url: None,
            copied: false,
            theme_state,
            theme_on_continue: false,
            rect_url: None,
            rect_token: None,
            rect_continue: None,
            rect_back: None,
            rect_themes: None,
        }
    }

    /// Limpia el estado de auth para un reintento (Back, o al volver a mostrar
    /// la pantalla tras borrar la última sesión). No toca el paso de tema.
    pub fn reset_auth(&mut self) {
        self.step = WelcomeStep::Auth;
        self.focus = WelcomeFocus::Url;
        self.token_input = TextInput::default();
        self.error = None;
        self.verifying = false;
        self.verifying_stored = false;
        self.oauth_in_progress = false;
        self.oauth_url = None;
        self.copied = false;
    }

    pub fn cycle_focus(&mut self, back: bool) {
        self.focus = if back {
            self.focus.prev()
        } else {
            self.focus.next()
        };
    }

    /// Mueve la selección de tema con envoltura; el llamador aplica el preview
    /// (`theme::set`). Devuelve el nuevo índice.
    pub fn move_theme(&mut self, delta: i32) -> usize {
        widgets::select_wrap(&mut self.theme_state, theme::all().len(), delta);
        self.theme_selected()
    }

    pub fn theme_selected(&self) -> usize {
        self.theme_state.selected().unwrap_or(0)
    }

    /// Entra al paso de tema tras un login exitoso en el primer arranque.
    pub fn enter_theme_step(&mut self) {
        self.step = WelcomeStep::Theme;
        self.theme_state.select(Some(theme::current_index()));
        self.theme_on_continue = false;
    }

    // --- Render ---

    pub fn draw(&mut self, frame: &mut Frame, area: Rect) {
        match self.step {
            WelcomeStep::Auth => self.draw_auth(frame, area),
            WelcomeStep::Theme => self.draw_theme(frame, area),
        }
    }

    fn draw_auth(&mut self, frame: &mut Frame, area: Rect) {
        let compact = area.width < 60 || area.height < 20;
        // Logo grande solo si el arte completo cabe junto al formulario.
        let big_logo = !compact
            && area.height >= GRID_ROWS as u16 + 18
            && area.width >= LOGO_COLS as u16;
        let logo_h: u16 = if big_logo { GRID_ROWS as u16 } else { 1 };
        let width: u16 = if compact { 50 } else { 72 };
        // blank+label+urlbox(6)+hint+blank+label+tokenbox(3)+blank+botones+blank+estado
        let total_h = logo_h + 18;
        let rect = layout::centered(area, width, total_h);
        frame.render_widget(Clear, rect);

        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(logo_h), // 0: logo
                Constraint::Length(1),      // 1: blank
                Constraint::Length(1),      // 2: label navegador
                Constraint::Length(6),      // 3: caja URL (4 líneas de contenido)
                Constraint::Length(1),      // 4: hint copiar
                Constraint::Length(1),      // 5: blank
                Constraint::Length(1),      // 6: label token
                Constraint::Length(3),      // 7: caja token
                Constraint::Length(1),      // 8: blank
                Constraint::Length(1),      // 9: botones
                Constraint::Length(1),      // 10: blank
                Constraint::Length(1),      // 11: estado
            ])
            .split(rect);

        // El logo es más ancho (87 cols) que la caja del formulario (72):
        // se centra sobre el ancho completo del frame, no sobre `rect`.
        let logo_area = Rect {
            x: area.x,
            y: rows[0].y,
            width: area.width,
            height: rows[0].height,
        };
        draw_logo(frame, logo_area, !big_logo);

        let browser_label = if self.oauth_in_progress {
            "Esperando autorización en el navegador…"
        } else {
            "Continue in browser"
        };
        frame.render_widget(
            Paragraph::new(Span::styled(browser_label, Style::default().fg(theme::fg())))
                .alignment(Alignment::Center),
            rows[2],
        );

        self.rect_url = Some(rows[3]);
        let url_body = match &self.oauth_url {
            Some(url) => vec![Line::from(Span::styled(
                url.clone(),
                Style::default().fg(theme::dim()),
            ))],
            None => vec![Line::from(Span::styled(
                "Generando URL…",
                Style::default().fg(theme::dim()),
            ))],
        };
        frame.render_widget(
            Paragraph::new(url_body)
                .block(Block::bordered().border_style(theme::border(self.focus == WelcomeFocus::Url)))
                .wrap(Wrap { trim: true }),
            rows[3],
        );
        let url_hint = if self.copied {
            "copiada ✓"
        } else {
            "Enter/click copiar"
        };
        frame.render_widget(
            Paragraph::new(Span::styled(url_hint, Style::default().fg(theme::dim())))
                .alignment(Alignment::Center),
            rows[4],
        );

        frame.render_widget(
            Paragraph::new(Span::styled(
                "Or, paste your Api Token manually:",
                Style::default().fg(theme::fg()),
            ))
            .alignment(Alignment::Center),
            rows[6],
        );

        self.rect_token = Some(rows[7]);
        let token_focused = self.focus == WelcomeFocus::Token;
        let masked = Line::from(widgets::masked_input_spans(&self.token_input, token_focused));
        frame.render_widget(
            Paragraph::new(masked)
                .block(Block::bordered().border_style(theme::border(token_focused))),
            rows[7],
        );

        let btn_cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(rows[9]);
        self.rect_continue = Some(btn_cols[0]);
        self.rect_back = Some(btn_cols[1]);
        frame.render_widget(
            button("Continuar", self.focus == WelcomeFocus::Continue),
            btn_cols[0],
        );
        frame.render_widget(button("Volver", self.focus == WelcomeFocus::Back), btn_cols[1]);

        let status = if self.verifying {
            Line::from(Span::styled(
                "Verificando…",
                Style::default().fg(theme::accent()),
            ))
        } else if let Some(err) = &self.error {
            Line::from(Span::styled(
                format!("✗ {err}"),
                Style::default().fg(theme::error()),
            ))
        } else {
            Line::from(Span::styled(
                "Tab/↑↓ mover · Enter aceptar · Esc reinicia · Ctrl-C salir",
                Style::default().fg(theme::dim()),
            ))
        };
        frame.render_widget(Paragraph::new(status).alignment(Alignment::Center), rows[11]);
    }

    fn draw_theme(&mut self, frame: &mut Frame, area: Rect) {
        let compact = area.width < 60 || area.height < 20;
        let list_h: u16 = theme::all().len() as u16 + 2;
        let big_logo = !compact
            && area.height >= GRID_ROWS as u16 + list_h + 5
            && area.width >= LOGO_COLS as u16;
        let logo_h: u16 = if big_logo { GRID_ROWS as u16 } else { 1 };
        let width: u16 = if compact { 50 } else { 72 };
        let total_h = logo_h + list_h + 5; // blank+título+lista+blank+botón+hint

        let rect = layout::centered(area, width, total_h);
        frame.render_widget(Clear, rect);

        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(logo_h),
                Constraint::Length(1),
                Constraint::Length(1),
                Constraint::Length(list_h),
                Constraint::Length(1),
                Constraint::Length(1),
                Constraint::Length(1),
            ])
            .split(rect);

        let logo_area = Rect {
            x: area.x,
            y: rows[0].y,
            width: area.width,
            height: rows[0].height,
        };
        draw_logo(frame, logo_area, !big_logo);

        frame.render_widget(
            Paragraph::new(Span::styled("Elige un tema", theme::title(true)))
                .alignment(Alignment::Center),
            rows[2],
        );

        self.rect_themes = Some(rows[3]);
        let items: Vec<ListItem> = theme::all()
            .iter()
            .enumerate()
            .map(|(i, t)| ListItem::new(theme_line(t, i == theme::current_index())))
            .collect();
        let list_focused = !self.theme_on_continue;
        let list = List::new(items)
            .block(Block::bordered().border_style(theme::border(list_focused)))
            .highlight_style(theme::selection())
            .highlight_symbol("▶ ");
        frame.render_stateful_widget(list, rows[3], &mut self.theme_state);

        self.rect_continue = Some(rows[5]);
        frame.render_widget(button("Continuar", self.theme_on_continue), rows[5]);

        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "↑↓ previsualizar · Tab botón · Enter/Esc confirmar",
                Style::default().fg(theme::dim()),
            )))
            .alignment(Alignment::Center),
            rows[6],
        );
    }
}

/// Botón de texto `[ Etiqueta ]`; resaltado (vídeo inverso) si tiene el foco.
fn button(label: &str, focused: bool) -> Paragraph<'static> {
    let style = if focused {
        Style::default()
            .fg(theme::accent())
            .add_modifier(Modifier::BOLD)
            .add_modifier(Modifier::REVERSED)
    } else {
        Style::default().fg(theme::fg())
    };
    Paragraph::new(Line::from(Span::styled(format!("[ {label} ]"), style)))
        .alignment(Alignment::Center)
}

fn draw_logo(frame: &mut Frame, area: Rect, compact: bool) {
    if compact {
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(
                    "lazy",
                    Style::default().fg(theme::fg()).add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    "cf",
                    Style::default()
                        .fg(theme::accent())
                        .add_modifier(Modifier::BOLD),
                ),
            ]))
            .alignment(Alignment::Center),
            area,
        );
    } else {
        frame.render_widget(
            Paragraph::new(logo_lines()).alignment(Alignment::Center),
            area,
        );
    }
}

// --- Logo pixel-art ---
//
// Arte fijo fila a fila (bloques `█` con remate `░`), tal cual el logo
// oficial. Todas las filas miden `LOGO_COLS` caracteres. Las columnas
// `< LOGO_SPLIT` son "lazy" (incluida la cola de la `y`, que cuelga bajo la
// `c`) y se pintan con `theme::fg()`; desde `LOGO_SPLIT` en adelante están
// la `c` y la `f`, en `theme::accent()`. Ningún tema pinta el fondo (ver
// `ui::theme`), así que los espacios quedan transparentes.

const GRID_ROWS: usize = 11;
const LOGO_COLS: usize = 87;
/// Columna (índice de carácter) donde termina "lazy" y empieza "cf".
const LOGO_SPLIT: usize = 57;

#[rustfmt::skip]
const LOGO: [&str; GRID_ROWS] = [
    "██████░                                                                       ██████░  ",
    "  ████░                                                                     ████░ ████░",
    "  ████░                                                                     ████░      ",
    "  ████░     ████████░     ██████████████░ ████░     ████░   ██████████░   ████████░    ",
    "  ████░           ████░   ██░     ████░   ████░     ████░ ████░     ████░   ████░      ",
    "  ████░     ██████████░         ████░     ████░     ████░ ████░             ████░      ",
    "  ████░   ████░   ████░       ████░       ████░     ████░ ████░             ████░      ",
    "  ████░   ████░   ████░     ████░     ██░   ████████████░ ████░     ████░   ████░      ",
    "████████░   ██████░ ████░ ██████████████░           ████░   ██████████░   ████████░    ",
    "                                                  ████░                                ",
    "                                          ██████████░                                  ",
];

fn logo_lines() -> Vec<Line<'static>> {
    let fg_style = Style::default().fg(theme::fg()).add_modifier(Modifier::BOLD);
    let accent_style = Style::default()
        .fg(theme::accent())
        .add_modifier(Modifier::BOLD);
    LOGO.iter()
        .map(|row| {
            let lazy: String = row.chars().take(LOGO_SPLIT).collect();
            let cf: String = row.chars().skip(LOGO_SPLIT).collect();
            Line::from(vec![
                Span::styled(lazy, fg_style),
                Span::styled(cf, accent_style),
            ])
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn logo_rows_have_expected_width() {
        for row in LOGO.iter() {
            assert_eq!(row.chars().count(), LOGO_COLS);
        }
    }

    #[test]
    fn logo_split_separates_lazy_from_cf() {
        // Ningún píxel de "cf" antes del corte en las filas del descendente de
        // la `y` (9-10), y la `c` arranca exactamente en el corte o después.
        for row in LOGO.iter() {
            let chars: Vec<char> = row.chars().collect();
            // El corte cae siempre en un espacio: nunca parte una letra.
            assert_eq!(chars[LOGO_SPLIT], ' ');
        }
    }

    #[test]
    fn logo_has_grid_rows_of_equal_width() {
        let lines = logo_lines();
        assert_eq!(lines.len(), GRID_ROWS);
        let w = lines[0].width();
        for l in &lines {
            assert_eq!(l.width(), w);
        }
    }
}
