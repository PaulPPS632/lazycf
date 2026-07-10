//! Barra inferior: estado a la izquierda, atajos de teclas a la derecha.

use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::Style;
use ratatui::widgets::Paragraph;

use crate::ui::theme;

pub struct CommandBar;

impl CommandBar {
    pub fn draw(&self, frame: &mut Frame, area: Rect, left: &str, right: &str) {
        let cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
            .split(area);
        frame.render_widget(
            Paragraph::new(format!(" {left}")).style(Style::default().fg(theme::fg())),
            cols[0],
        );
        frame.render_widget(
            Paragraph::new(format!("{right} "))
                .alignment(Alignment::Right)
                .style(Style::default().fg(theme::dim())),
            cols[1],
        );
    }
}
