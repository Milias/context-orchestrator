use crate::tui::{FocusPanel, TuiState};
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

pub fn render(frame: &mut Frame, area: Rect, frame_area: Rect, tui_state: &TuiState) {
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

    // Autocomplete popup
    if tui_state.autocomplete.active && !tui_state.autocomplete.candidates.is_empty() {
        render_autocomplete_popup(frame, area, frame_area, cursor_x, tui_state);
    }
}

fn render_autocomplete_popup(
    frame: &mut Frame,
    input_area: Rect,
    frame_area: Rect,
    cursor_x: u16,
    tui_state: &TuiState,
) {
    let candidates = &tui_state.autocomplete.candidates;
    let max_visible: usize = 5;
    let visible_count = candidates.len().min(max_visible);

    // Calculate popup dimensions
    let content_width = candidates
        .iter()
        .map(|c| c.name.len() + 2 + c.description.len())
        .max()
        .unwrap_or(10);

    // +2 for borders, +2 for padding
    #[allow(clippy::cast_possible_truncation)]
    let popup_width = (content_width as u16 + 4).min(frame_area.width.saturating_sub(2));
    #[allow(clippy::cast_possible_truncation)]
    let popup_height = visible_count as u16 + 2; // +2 for borders

    // Position: above input box, anchored near cursor
    let x = cursor_x.min(frame_area.width.saturating_sub(popup_width));
    let y = input_area.y.saturating_sub(popup_height);

    let popup_area = Rect {
        x,
        y,
        width: popup_width,
        height: popup_height,
    };

    frame.render_widget(Clear, popup_area);

    let lines: Vec<Line> = candidates
        .iter()
        .take(max_visible)
        .enumerate()
        .map(|(i, c)| {
            let is_selected = i == tui_state.autocomplete.selected;
            let base = if is_selected {
                Style::default().bg(Color::DarkGray).fg(Color::White)
            } else {
                Style::default()
            };
            Line::from(vec![
                Span::styled(format!(" {} ", c.name), base.add_modifier(Modifier::BOLD)),
                Span::styled(&c.description, base.fg(Color::Gray)),
            ])
        })
        .collect();

    let popup = Paragraph::new(lines).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Yellow))
            .title("/commands"),
    );

    frame.render_widget(popup, popup_area);
}
