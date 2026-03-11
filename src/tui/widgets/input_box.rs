use crate::tui::{FocusPanel, TuiState};
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Paragraph};

pub fn render(frame: &mut Frame, area: Rect, tui_state: &TuiState) {
    let border_color = if tui_state.focus == FocusPanel::Input {
        Color::Yellow
    } else {
        Color::DarkGray
    };

    let input = Paragraph::new(tui_state.input_text.as_str()).block(
        Block::default()
            .title("Message (Enter: send | Tab: focus | Ctrl+B: panel | Ctrl+Q: quit)")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(border_color)),
    );

    frame.render_widget(input, area);

    // Cursor position within a terminal row; input length is bounded by screen width so
    // truncation to u16 is safe.
    #[allow(clippy::cast_possible_truncation)]
    let cursor_x = area.x + 1 + tui_state.input_cursor as u16;
    let cursor_y = area.y + 1;
    frame.set_cursor_position((cursor_x, cursor_y));
}
