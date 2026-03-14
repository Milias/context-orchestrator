use crate::tui::input::cursor_line_col;
use crate::tui::state::FocusZone;
use crate::tui::TuiState;
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

/// Render the persistent input box at the bottom of the screen.
/// Shows answer mode (cyan border, question text) when a user question is pending.
/// Scrolls vertically to keep the cursor visible when text exceeds the box height.
pub fn render(frame: &mut Frame, area: Rect, frame_area: Rect, tui_state: &mut TuiState) {
    let (title, border_color) = if let Some(ref q) = tui_state.pending_question_text {
        let max_chars = 60;
        let char_count = q.chars().count();
        let truncated = if char_count > max_chars {
            let taken: String = q.chars().take(max_chars - 3).collect();
            format!("{taken}...")
        } else {
            q.clone()
        };
        (format!("Answer: {truncated}"), Color::Cyan)
    } else {
        let color = if tui_state.nav.focus == FocusZone::ChatPanel {
            Color::Yellow
        } else {
            Color::DarkGray
        };
        ("Message".to_string(), color)
    };

    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color));

    // Compute scroll offset to keep cursor visible within the inner area.
    let inner_height = area.height.saturating_sub(2) as usize; // -2 for borders
    let (cursor_line, col) = cursor_line_col(tui_state.input.text(), tui_state.input.cursor());
    let scroll = &mut tui_state.input_scroll;
    if inner_height > 0 {
        // Scroll offsets are bounded by terminal height (u16 max).
        #[allow(clippy::cast_possible_truncation)]
        if cursor_line >= *scroll as usize + inner_height {
            *scroll = (cursor_line + 1).saturating_sub(inner_height) as u16;
        } else if cursor_line < *scroll as usize {
            #[allow(clippy::cast_possible_truncation)]
            {
                *scroll = cursor_line as u16;
            }
        }
    }

    let input = Paragraph::new(tui_state.input.text())
        .block(block)
        .scroll((tui_state.input_scroll, 0));

    frame.render_widget(input, area);

    // Cursor position adjusted for scroll offset.
    #[allow(clippy::cast_possible_truncation)] // bounded by terminal width
    let cursor_x = area.x + 1 + col as u16;
    #[allow(clippy::cast_possible_truncation)] // bounded by input box height
    let cursor_y = area.y + 1 + (cursor_line as u16).saturating_sub(tui_state.input_scroll);
    frame.set_cursor_position((cursor_x, cursor_y));

    // Autocomplete popup
    if tui_state.autocomplete.active && !tui_state.autocomplete.candidates.is_empty() {
        render_autocomplete_popup(frame, area, frame_area, cursor_x, tui_state);
    }
}

/// Compute the height the input box should occupy, based on text content.
///
/// Accounts for explicit newlines and soft wrapping at the given width.
/// Returns a value between `MIN_HEIGHT` and `max_height`.
pub fn compute_height(tui_state: &TuiState, content_width: u16, max_height: u16) -> u16 {
    const MIN_HEIGHT: u16 = 3; // 1 line + 2 borders
    const BORDER_ROWS: u16 = 2;

    let inner_width = content_width.saturating_sub(BORDER_ROWS) as usize;
    // Visual line count is bounded by text length; terminal height caps it well under u16::MAX.
    #[allow(clippy::cast_possible_truncation)]
    let visual_lines = tui_state.input.visual_line_count(inner_width) as u16;
    (visual_lines + BORDER_ROWS).clamp(MIN_HEIGHT, max_height.max(MIN_HEIGHT))
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
    // content_width is bounded by terminal width (u16), so truncation is safe.
    #[allow(clippy::cast_possible_truncation)]
    let popup_width = (content_width as u16 + 4).min(frame_area.width.saturating_sub(2));
    // visible_count is capped at max_visible (5), so truncation is safe.
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
