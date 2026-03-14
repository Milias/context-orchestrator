//! Top-level rendering: tab bar + master horizontal split + status bar.
//!
//! The left content area is controlled by `nav.active_tab` (placeholder
//! until per-tab renderers are built). The right panel is the persistent
//! conversation widget, toggleable with `Ctrl+B`.

use crate::graph::ConversationGraph;
use crate::tui::state::FocusZone;
use crate::tui::tabs;
use crate::tui::widgets::{conversation, input_box};
use crate::tui::TuiState;
use ratatui::prelude::*;
use ratatui::widgets::Paragraph;

/// Minimum terminal width for the conversation panel to be visible by default.
const WIDE_THRESHOLD: u16 = 120;

pub fn draw(frame: &mut Frame, graph: &ConversationGraph, tui_state: &mut TuiState) {
    let area = frame.area();

    // Auto-hide conversation on narrow terminals.
    let show_conversation = tui_state.nav.conversation_visible && area.width >= WIDE_THRESHOLD || {
        // User can force-toggle even on narrow terminals.
        tui_state.nav.conversation_visible && area.width >= 80
    };

    // Vertical split: tab bar (1) | content (flex) | status bar (1) | input (3).
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(5),
            Constraint::Length(1),
            Constraint::Length(3),
        ])
        .split(area);

    let tab_bar_area = vertical[0];
    let content_area = vertical[1];
    let status_bar_area = vertical[2];
    let input_area = vertical[3];

    // Tab bar with status info merged into the right side.
    draw_tab_status_bar(frame, tab_bar_area, graph, tui_state);

    // Master horizontal split: left (tab content) | right (conversation).
    if show_conversation {
        let horizontal = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
            .split(content_area);

        render_tab_content(frame, horizontal[0], graph, tui_state);
        conversation::render(frame, horizontal[1], graph, tui_state);
    } else {
        render_tab_content(frame, content_area, graph, tui_state);
    }

    // Status bar below content.
    draw_status_bar(frame, status_bar_area, tui_state);

    // Persistent input box.
    input_box::render(frame, input_area, area, tui_state);
}

/// Dispatch to the active tab's renderer (placeholders for now).
fn render_tab_content(
    frame: &mut Frame,
    area: Rect,
    _graph: &ConversationGraph,
    tui_state: &TuiState,
) {
    tabs::render_placeholder(frame, area, tui_state.nav.active_tab);
}

/// Combined tab bar + branch/token info in a single row.
fn draw_tab_status_bar(
    frame: &mut Frame,
    area: Rect,
    graph: &ConversationGraph,
    tui_state: &TuiState,
) {
    let bg = Style::default().bg(Color::Rgb(30, 30, 80));
    let mut spans: Vec<Span> = Vec::new();

    // Tab labels.
    for (i, tab) in crate::tui::state::TopTab::all().iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled(" | ", bg.fg(Color::DarkGray)));
        }
        let style = if *tab == tui_state.nav.active_tab {
            bg.fg(Color::White).add_modifier(Modifier::BOLD)
        } else {
            bg.fg(Color::DarkGray)
        };
        spans.push(Span::styled(tab.label(), style));
    }

    // Right-aligned: branch + tokens + errors.
    let branch = graph.active_branch().to_string();
    let input_tok = tui_state.token_usage.input.current;
    let output_tok = tui_state.token_usage.output.current;
    let token_text = if input_tok > 0 || output_tok > 0 {
        format!(
            "{}in / {}out",
            format_token_count(input_tok),
            format_token_count(output_tok),
        )
    } else {
        String::new()
    };
    let right = if token_text.is_empty() {
        format!("[{branch}]")
    } else {
        format!("[{branch}]  {token_text}")
    };

    let left_width: usize = spans.iter().map(Span::width).sum();
    let pad = (area.width as usize).saturating_sub(left_width + right.len());
    spans.push(Span::styled(" ".repeat(pad), bg));
    spans.push(Span::styled(right, bg.fg(Color::DarkGray)));

    let line = Line::from(spans);
    frame.render_widget(Paragraph::new(line).style(bg), area);
}

/// Minimal status bar showing focus zone, status message, and errors.
fn draw_status_bar(frame: &mut Frame, area: Rect, tui_state: &TuiState) {
    let bg = Style::default().bg(Color::Rgb(20, 20, 50));

    let focus_label = match tui_state.nav.focus {
        FocusZone::TabContent => tui_state.nav.active_tab.label(),
        FocusZone::Conversation => "Chat",
        FocusZone::Input => "Input",
    };

    let left = tui_state
        .status_message
        .as_deref()
        .unwrap_or(focus_label)
        .to_string();

    let error_text = tui_state
        .error_message
        .as_ref()
        .map_or(String::new(), Clone::clone);

    let width = area.width as usize;
    let max_left = width.saturating_sub(error_text.len() + 1);
    let left_display: String = if left.len() > max_left && max_left > 1 {
        let truncated: String = left.chars().take(max_left - 1).collect();
        format!("{truncated}…")
    } else {
        left
    };

    let pad = width.saturating_sub(left_display.len() + error_text.len());
    let mut spans = vec![
        Span::styled(left_display, bg.fg(Color::White)),
        Span::styled(" ".repeat(pad), bg),
    ];
    if !error_text.is_empty() {
        spans.push(Span::styled(error_text, bg.fg(Color::Red)));
    }

    let line = Line::from(spans);
    frame.render_widget(Paragraph::new(line).style(bg), area);
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
