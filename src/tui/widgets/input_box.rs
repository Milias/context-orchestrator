use crate::tui::{Focus, TuiState};
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Paragraph};

pub fn render(frame: &mut Frame, area: Rect, tui_state: &TuiState) {
    let (title, text) = if tui_state.naming_branch {
        (
            "Branch name (Enter: confirm | Esc: cancel)",
            &tui_state.branch_name_input,
        )
    } else {
        (
            "Message (Enter: send | Ctrl+B: branch | Tab: focus | Ctrl+Q: quit)",
            &tui_state.input_text,
        )
    };

    let border_style = if tui_state.focus == Focus::Input {
        Style::default().fg(Color::Yellow)
    } else {
        Style::default()
    };

    let input = Paragraph::new(text.as_str()).block(
        Block::default()
            .title(title)
            .borders(Borders::ALL)
            .border_style(border_style),
    );

    frame.render_widget(input, area);

    // Position cursor
    if tui_state.focus == Focus::Input {
        let cursor_pos = if tui_state.naming_branch {
            tui_state.branch_name_input.chars().count()
        } else {
            tui_state.input_cursor
        };
        let cursor_x = area.x + 1 + cursor_pos as u16;
        let cursor_y = area.y + 1;
        frame.set_cursor_position((cursor_x, cursor_y));
    }
}
