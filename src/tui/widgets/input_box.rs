use crate::tui::TuiState;
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Paragraph};

pub fn render(frame: &mut Frame, area: Rect, tui_state: &TuiState) {
    let input = Paragraph::new(tui_state.input_text.as_str()).block(
        Block::default()
            .title("Message (Enter: send | Ctrl+Q: quit)")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Yellow)),
    );

    frame.render_widget(input, area);

    let cursor_x = area.x + 1 + tui_state.input_cursor as u16;
    let cursor_y = area.y + 1;
    frame.set_cursor_position((cursor_x, cursor_y));
}
