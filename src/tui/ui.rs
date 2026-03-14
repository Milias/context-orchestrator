use crate::graph::ConversationGraph;
use crate::tui::widgets::{context_panel, conversation, input_box};
use crate::tui::TuiState;
use ratatui::prelude::*;
use ratatui::widgets::Paragraph;

pub fn draw(frame: &mut Frame, graph: &ConversationGraph, tui_state: &mut TuiState) {
    let area = frame.area();
    let show_context = tui_state.context_panel_visible && area.height >= 20;

    let vertical = if show_context {
        Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),
                Constraint::Percentage(30),
                Constraint::Min(5),
                Constraint::Length(5),
            ])
            .split(area)
    } else {
        Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),
                Constraint::Min(5),
                Constraint::Length(5),
            ])
            .split(area)
    };

    if show_context {
        draw_status_bar(frame, vertical[0], graph, tui_state);
        context_panel::render(frame, vertical[1], graph, tui_state);
        conversation::render(frame, vertical[2], graph, tui_state);
        input_box::render(frame, vertical[3], area, tui_state);
    } else {
        draw_status_bar(frame, vertical[0], graph, tui_state);
        conversation::render(frame, vertical[1], graph, tui_state);
        input_box::render(frame, vertical[2], area, tui_state);
    }
}

fn draw_status_bar(frame: &mut Frame, area: Rect, graph: &ConversationGraph, tui_state: &TuiState) {
    let left_text = tui_state
        .status_message
        .as_deref()
        .unwrap_or("")
        .to_string();
    let left_text = if left_text.is_empty() {
        format!("Context Manager v0.1  [branch: {}]", graph.active_branch())
    } else {
        left_text
    };

    let bg = Style::default().bg(Color::Rgb(30, 30, 80));

    // Build right-aligned token counter text.
    let input = tui_state.token_usage.input.current;
    let output = tui_state.token_usage.output.current;
    let token_text = if input > 0 || output > 0 {
        format!(
            "{}in / {}out",
            format_token_count(input),
            format_token_count(output),
        )
    } else {
        String::new()
    };

    // Build right-aligned error text.
    let error_text = tui_state
        .error_message
        .as_ref()
        .map_or(String::new(), |e| format!("  {e}"));

    // Truncate left text if necessary to preserve right-aligned content.
    let right_len = token_text.len() + error_text.len();
    let width = area.width as usize;
    let max_left = width.saturating_sub(right_len + 1); // +1 for min padding
    let left_display: String = if left_text.len() > max_left && max_left > 1 {
        let truncated: String = left_text.chars().take(max_left - 1).collect();
        format!("{truncated}…")
    } else {
        left_text
    };

    let pad = width.saturating_sub(left_display.len() + right_len);

    let mut spans = vec![
        Span::styled(left_display, bg.fg(Color::White)),
        Span::styled(" ".repeat(pad), bg),
    ];
    if !token_text.is_empty() {
        spans.push(Span::styled(token_text, bg.fg(Color::DarkGray)));
    }
    if !error_text.is_empty() {
        spans.push(Span::styled(error_text, bg.fg(Color::Red)));
    }

    let line = Line::from(spans);
    let status = Paragraph::new(line).style(bg);
    frame.render_widget(status, area);
}

/// Format a token count for compact display in the status bar.
///
/// Returns `"1.2M"` for millions, `"45.3k"` for thousands, or the
/// raw number for values under 1 000.
// Precision loss is acceptable: at u64::MAX (~18.4 quintillion tokens)
// the error is < 0.1%, and realistic token counts are well under 2^52.
#[allow(clippy::cast_precision_loss)]
pub(crate) fn format_token_count(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.1}k", n as f64 / 1_000.0)
    } else {
        n.to_string()
    }
}

#[cfg(test)]
#[path = "ui_tests.rs"]
mod tests;
