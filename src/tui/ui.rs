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

pub fn draw(frame: &mut Frame, graph: &ConversationGraph, tui_state: &mut TuiState) {
    let area = frame.area();

    // Conversation panel visible when toggled on and terminal >= 80 cols.
    let show_conversation = tui_state.nav.conversation_visible && area.width >= 80;

    // Outer vertical: tab bar (1) | main content (flex) | status bar (1).
    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(5),
            Constraint::Length(1),
        ])
        .split(area);

    draw_tab_status_bar(frame, outer[0], graph, tui_state);
    draw_status_bar(frame, outer[2], tui_state);

    // Horizontal: left (tab content) | right (conversation + input).
    if show_conversation {
        let horizontal = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(75), Constraint::Percentage(25)])
            .split(outer[1]);

        render_tab_content(frame, horizontal[0], graph, tui_state);

        // Right column: conversation (flex) + input (3 rows).
        let right_col = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(3), Constraint::Length(3)])
            .split(horizontal[1]);

        conversation::render(frame, right_col[0], graph, tui_state);
        input_box::render(frame, right_col[1], area, tui_state);
    } else {
        // No conversation: tab content fills the area, no input box visible.
        render_tab_content(frame, outer[1], graph, tui_state);
    }
}

/// Dispatch to the active tab's renderer.
fn render_tab_content(
    frame: &mut Frame,
    area: Rect,
    graph: &ConversationGraph,
    tui_state: &mut TuiState,
) {
    match tui_state.nav.active_tab {
        crate::tui::state::TopTab::Agents => {
            tabs::agents::render(frame, area, graph, tui_state);
        }
        crate::tui::state::TopTab::Work => {
            tabs::work::render(frame, area, graph, tui_state);
        }
        crate::tui::state::TopTab::Activity => {
            tabs::activity::render(frame, area, graph, tui_state);
        }
    }
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

/// Status bar with context-aware shortcuts and errors.
fn draw_status_bar(frame: &mut Frame, area: Rect, tui_state: &TuiState) {
    let bg = Style::default().bg(Color::Rgb(20, 20, 50));
    let dim = bg.fg(Color::DarkGray);
    let key_style = bg.fg(Color::Cyan);

    // Left: context-aware shortcuts.
    let shortcuts = build_shortcuts(tui_state);
    let mut spans: Vec<Span> = Vec::new();
    for (i, (key, desc)) in shortcuts.iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled("  ", dim));
        }
        spans.push(Span::styled(*key, key_style));
        spans.push(Span::styled(format!(":{desc}"), dim));
    }

    // Right: error text.
    let error_text = tui_state
        .error_message
        .as_ref()
        .map_or(String::new(), Clone::clone);

    let left_width: usize = spans.iter().map(Span::width).sum();
    let width = area.width as usize;
    let pad = width.saturating_sub(left_width + error_text.len());
    spans.push(Span::styled(" ".repeat(pad), bg));
    if !error_text.is_empty() {
        spans.push(Span::styled(error_text, bg.fg(Color::Red)));
    }

    let line = Line::from(spans);
    frame.render_widget(Paragraph::new(line).style(bg), area);
}

/// Build context-aware shortcut hints based on the current focus zone.
fn build_shortcuts(tui_state: &TuiState) -> Vec<(&'static str, &'static str)> {
    let mut shortcuts = vec![
        ("1-3", "view"),
        ("Tab", "focus"),
        ("Ctrl+B", "chat"),
        ("Ctrl+Q", "quit"),
    ];

    match tui_state.nav.focus {
        FocusZone::Conversation => {
            shortcuts.insert(0, ("Ctrl+E", "tools"));
            shortcuts.insert(0, ("End", "auto-scroll"));
            shortcuts.insert(0, ("Up/Dn", "scroll"));
        }
        FocusZone::Input => {
            shortcuts.insert(0, ("Enter", "send"));
        }
        FocusZone::TabContent => {
            shortcuts.insert(0, ("Up/Dn", "nav"));
        }
    }
    shortcuts
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
