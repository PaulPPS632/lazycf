//! Panel principal: detalle del módulo seleccionado. En Fase 0 muestra un
//! placeholder por módulo; cada fase posterior lo reemplaza por su vista real.

use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph, Wrap};
use ratatui::Frame;

use super::Module;
use crate::ui::theme;

pub struct Detail;

impl Detail {
    pub fn new() -> Self {
        Self
    }

    pub fn draw(&self, frame: &mut Frame, area: Rect, module: Module, focused: bool) {
        let block = Block::bordered()
            .title(format!(" {} {} ", module.icon(), module.label()))
            .border_style(theme::border(focused))
            .title_style(theme::title(focused));
        let body = Paragraph::new(vec![
            Line::from(Span::styled(module.hint(), theme::title(false))),
            Line::from(""),
            Line::from("(módulo en construcción — próxima fase)"),
        ])
        .block(block)
        .wrap(Wrap { trim: true });
        frame.render_widget(body, area);
    }
}
