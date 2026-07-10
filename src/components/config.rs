//! Pantalla de configuración full-screen (tecla `,`): sidebar de secciones +
//! columna de contenido. Reutiliza el `layout::shell` de la pantalla principal.
//! El estado de tema/cuentas lo posee `app.rs`; aquí vive el estado de la vista
//! (foco, sección, selección) y el render.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, List, ListItem, ListState, Paragraph, Wrap};

use crate::components::popup::AccountRow;
use crate::ui::theme;
use crate::ui::widgets::{select_wrap, theme_line};

/// Secciones de la pantalla de configuración.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigSection {
    Theme,
    Accounts,
    Language,
}

impl ConfigSection {
    pub const ALL: [ConfigSection; 3] = [
        ConfigSection::Theme,
        ConfigSection::Accounts,
        ConfigSection::Language,
    ];

    pub fn label(self) -> &'static str {
        match self {
            ConfigSection::Theme => "Tema",
            ConfigSection::Accounts => "Cuentas",
            ConfigSection::Language => "Idioma",
        }
    }
}

/// Columna con foco dentro de la pantalla de config.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigFocus {
    Sections,
    Content,
}

/// Estado de la pantalla de configuración.
pub struct ConfigView {
    pub focus: ConfigFocus,
    pub section: ConfigSection,
    sections_state: ListState,
    theme_state: ListState,
    /// Índice del tema guardado al abrir: marca el `●` y define qué restaurar
    /// si el usuario sale sin confirmar un preview.
    pub saved_theme: usize,
    accounts: Vec<AccountRow>,
    accounts_state: ListState,
}

impl ConfigView {
    pub fn new() -> Self {
        let mut sections_state = ListState::default();
        sections_state.select(Some(0));
        Self {
            focus: ConfigFocus::Sections,
            section: ConfigSection::Theme,
            sections_state,
            theme_state: ListState::default(),
            saved_theme: 0,
            accounts: Vec::new(),
            accounts_state: ListState::default(),
        }
    }

    /// Prepara la vista al abrir: resetea el foco/sección, fija el tema guardado
    /// (para el `●` y el preview) y carga las cuentas.
    pub fn open(&mut self, saved_theme: usize, accounts: Vec<AccountRow>) {
        self.focus = ConfigFocus::Sections;
        self.section = ConfigSection::Theme;
        self.sections_state.select(Some(0));
        self.saved_theme = saved_theme;
        self.theme_state.select(Some(saved_theme));
        self.set_accounts(accounts);
    }

    /// Refresca las cuentas (tras añadir/borrar/cambiar), conservando la
    /// selección en la cuenta activa.
    pub fn set_accounts(&mut self, accounts: Vec<AccountRow>) {
        let sel = accounts.iter().position(|r| r.active).unwrap_or(0);
        self.accounts_state
            .select((!accounts.is_empty()).then_some(sel));
        self.accounts = accounts;
    }

    /// Mueve la selección de secciones (sidebar) con envoltura.
    pub fn move_section(&mut self, delta: i32) {
        if select_wrap(&mut self.sections_state, ConfigSection::ALL.len(), delta) {
            self.section = ConfigSection::ALL[self.sections_state.selected().unwrap_or(0)];
        }
    }

    /// Selecciona la sección por fila relativa (click). `true` si cambió.
    pub fn section_at(&mut self, rel: usize) -> bool {
        if rel >= ConfigSection::ALL.len() {
            return false;
        }
        let changed = self.section != ConfigSection::ALL[rel];
        self.sections_state.select(Some(rel));
        self.section = ConfigSection::ALL[rel];
        changed
    }

    /// Alterna el foco entre la lista de secciones y el contenido.
    pub fn toggle_focus(&mut self) {
        self.focus = match self.focus {
            ConfigFocus::Sections => ConfigFocus::Content,
            ConfigFocus::Content => ConfigFocus::Sections,
        };
    }

    // --- Tema ---

    /// Índice del tema seleccionado en la lista.
    pub fn theme_selected(&self) -> usize {
        self.theme_state.selected().unwrap_or(0)
    }

    /// Mueve la selección de tema con envoltura y devuelve el nuevo índice.
    pub fn move_theme(&mut self, delta: i32) -> usize {
        select_wrap(&mut self.theme_state, theme::all().len(), delta);
        self.theme_selected()
    }

    // --- Cuentas ---

    pub fn move_account(&mut self, delta: i32) {
        select_wrap(&mut self.accounts_state, self.accounts.len(), delta);
    }

    pub fn selected_account(&self) -> Option<&AccountRow> {
        self.accounts_state
            .selected()
            .and_then(|i| self.accounts.get(i))
    }

    // --- Render ---

    /// Sidebar con la lista de secciones.
    pub fn draw_sections(&mut self, frame: &mut Frame, area: Rect, focused: bool) {
        let items: Vec<ListItem> = ConfigSection::ALL
            .iter()
            .map(|s| ListItem::new(format!(" {}", s.label())))
            .collect();
        let list = List::new(items)
            .block(
                Block::bordered()
                    .title(" Configuración ")
                    .border_style(theme::border(focused))
                    .title_style(theme::title(focused)),
            )
            .highlight_style(theme::selection())
            .highlight_symbol("▶ ");
        frame.render_stateful_widget(list, area, &mut self.sections_state);
    }

    /// Columna de contenido de la sección activa.
    pub fn draw_content(&mut self, frame: &mut Frame, area: Rect, focused: bool) {
        match self.section {
            ConfigSection::Theme => self.draw_theme_content(frame, area, focused),
            ConfigSection::Accounts => self.draw_accounts_content(frame, area, focused),
            ConfigSection::Language => self.draw_language_content(frame, area, focused),
        }
    }

    fn draw_theme_content(&mut self, frame: &mut Frame, area: Rect, focused: bool) {
        let items: Vec<ListItem> = theme::all()
            .iter()
            .enumerate()
            .map(|(i, t)| ListItem::new(theme_line(t, i == self.saved_theme)))
            .collect();
        let list = List::new(items)
            .block(
                Block::bordered()
                    .title(" Tema ")
                    .border_style(theme::border(focused))
                    .title_style(theme::title(focused)),
            )
            .highlight_style(theme::selection())
            .highlight_symbol("▶ ");
        frame.render_stateful_widget(list, area, &mut self.theme_state);
    }

    fn draw_accounts_content(&mut self, frame: &mut Frame, area: Rect, focused: bool) {
        let block = Block::bordered()
            .title(" Cuentas ")
            .border_style(theme::border(focused))
            .title_style(theme::title(focused));
        if self.accounts.is_empty() {
            let body = Paragraph::new("Sin cuentas · pulsa 'a' para añadir un token")
                .block(block)
                .style(Style::default().fg(theme::dim()));
            frame.render_widget(body, area);
            return;
        }
        // Mismo render que el selector de cuentas (popup): ● = cuenta activa.
        let items: Vec<ListItem> = self
            .accounts
            .iter()
            .map(|r| {
                let marker = if r.active { "● " } else { "  " };
                let style = if r.active {
                    Style::default().fg(theme::accent())
                } else {
                    Style::default().fg(theme::fg())
                };
                ListItem::new(Line::from(Span::styled(
                    format!("{marker}{}", r.label),
                    style,
                )))
            })
            .collect();
        let list = List::new(items)
            .block(block)
            .highlight_style(theme::selection())
            .highlight_symbol("▶ ");
        frame.render_stateful_widget(list, area, &mut self.accounts_state);
    }

    fn draw_language_content(&mut self, frame: &mut Frame, area: Rect, focused: bool) {
        let block = Block::bordered()
            .title(" Idioma ")
            .border_style(theme::border(focused))
            .title_style(theme::title(focused));
        let body = Paragraph::new("Próximamente · la interfaz está en español")
            .block(block)
            .style(Style::default().fg(theme::dim()))
            .wrap(Wrap { trim: true });
        frame.render_widget(body, area);
    }
}
