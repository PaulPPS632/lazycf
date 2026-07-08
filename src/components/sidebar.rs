//! Panel izquierdo: lista navegable de los módulos de Cloudflare.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::Rect;
use ratatui::widgets::{Block, List, ListItem, ListState};
use ratatui::Frame;

use super::{Component, Module};
use crate::action::Action;
use crate::ui::theme;

pub struct Sidebar {
    pub selected: usize,
    state: ListState,
}

impl Sidebar {
    pub fn new() -> Self {
        let mut state = ListState::default();
        state.select(Some(0));
        Self { selected: 0, state }
    }

    /// Módulo actualmente seleccionado.
    pub fn module(&self) -> Module {
        Module::ALL[self.selected]
    }

    /// Mueve la selección (para scroll de mouse).
    pub fn move_by(&mut self, delta: i32) {
        let n = Module::ALL.len() as i32;
        self.selected = ((((self.selected as i32 + delta) % n) + n) % n) as usize;
        self.state.select(Some(self.selected));
    }

    /// Selecciona por fila relativa (click); `true` si cayó en un módulo.
    pub fn module_at(&mut self, rel: usize) -> bool {
        let idx = rel + self.state.offset();
        if idx < Module::ALL.len() {
            self.selected = idx;
            self.state.select(Some(idx));
            true
        } else {
            false
        }
    }

    /// Cambia el módulo activo por programa (navegación cruzada de módulos).
    pub fn set_module(&mut self, m: Module) {
        if let Some(idx) = Module::ALL.iter().position(|x| *x == m) {
            self.selected = idx;
            self.state.select(Some(idx));
        }
    }
}

impl Component for Sidebar {
    fn handle_key(&mut self, key: KeyEvent) -> Option<Action> {
        let n = Module::ALL.len();
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => {
                self.selected = (self.selected + n - 1) % n;
                None
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.selected = (self.selected + 1) % n;
                None
            }
            _ => None,
        }
    }

    fn draw(&mut self, frame: &mut Frame, area: Rect, focused: bool) {
        let items: Vec<ListItem> = Module::ALL
            .iter()
            .map(|m| ListItem::new(format!(" {}  {}", m.icon(), m.label())))
            .collect();
        self.state.select(Some(self.selected));
        let list = List::new(items)
            .block(
                Block::bordered()
                    .title(" Módulos ")
                    .border_style(theme::border(focused))
                    .title_style(theme::title(focused)),
            )
            .highlight_style(theme::selection())
            .highlight_symbol("▶ ");
        frame.render_stateful_widget(list, area, &mut self.state);
    }
}
