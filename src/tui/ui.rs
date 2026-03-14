//! Top-level rendering: agent header + tab bar + master horizontal split + status bar.
//!
//! Layout structure:
//! ```text
//! Outer vertical: agent_header (dynamic) | main_content (flex) | status_bar (1)
//! Main horizontal: left_panel (65%) | right_panel (35%)
//! Left vertical: tab_bar (1) | search_bar (0 or 1) | tab_content (flex)
//! Right vertical: conversation (flex) | input_box (3..dynamic)
//! ```
//!
//! The conversation panel is always visible on the right.
//! The agent header expands from 0 lines when agents are active.
//! The search bar appears below the tab bar when a search is active.

use crate::graph::ConversationGraph;
use crate::tui::state::FocusZone;
use crate::tui::tabs;
use crate::tui::widgets::{conversation, input_box};
use crate::tui::TuiState;
use ratatui::prelude::*;
use ratatui::widgets::Paragraph;

/// Draw the full TUI frame.
///
/// Always renders:
/// - Agent header (dynamic 0..N lines, only when agents active)
/// - Left panel (tab bar + tab content) at ~65% width
/// - Right panel (conversation + input) at ~35% width
/// - Status bar at bottom
pub fn draw(frame: &mut Frame, graph: &ConversationGraph, tui_state: &mut TuiState) {
    let area = frame.area();

    let agent_header_h = compute_agent_header_height(tui_state);

    // Outer vertical: agent_header (dynamic) | main_content (flex) | status_bar (1).
    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(agent_header_h),
            Constraint::Min(5),
            Constraint::Length(1),
        ])
        .split(area);

    if agent_header_h > 0 {
        render_agent_header(frame, outer[0], tui_state);
    }
    draw_status_bar(frame, outer[2], graph, tui_state);

    // Main horizontal: left_panel (65%) | right_panel (35%).
    let horizontal = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(65), Constraint::Percentage(35)])
        .split(outer[1]);

    // Left vertical: tab_bar (1) | search_bar (0 or 1) | tab_content (flex).
    let search_bar_h: u16 = u16::from(tui_state.search.is_some());
    let left_col = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(search_bar_h),
            Constraint::Min(3),
        ])
        .split(horizontal[0]);

    render_tab_bar(frame, left_col[0], tui_state);
    if tui_state.search.is_some() {
        render_search_bar(frame, left_col[1], tui_state);
    }
    render_tab_content(frame, left_col[2], graph, tui_state);

    // Right column: conversation (flex) + input (dynamic height).
    let max_input = (horizontal[1].height * 40 / 100).max(3);
    let input_height = input_box::compute_height(tui_state, horizontal[1].width, max_input);
    let right_col = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(3), Constraint::Length(input_height)])
        .split(horizontal[1]);

    conversation::render(frame, right_col[0], graph, tui_state);
    tui_state.panel_rects.conversation = right_col[0];
    input_box::render(frame, right_col[1], area, tui_state);
}

/// Compute agent header height. Returns 0 when no agents are active,
/// otherwise 2 lines per agent + spacing between agents.
fn compute_agent_header_height(tui_state: &TuiState) -> u16 {
    let count = tui_state.agent_displays.len();
    if count == 0 {
        return 0;
    }
    // 2 lines per agent (phase + detail) + (count-1) blank lines between.
    let inner = 2 * count + count.saturating_sub(1);
    // Cast safety: agent count is small (<10), so inner << u16::MAX.
    #[allow(clippy::cast_possible_truncation)]
    let h = inner as u16;
    h.max(2)
}

/// Render the agent header strip. Only called when agents are active.
/// Reuses the same visual format as the overview agent card but without borders.
fn render_agent_header(frame: &mut Frame, area: Rect, tui_state: &TuiState) {
    let bg = Style::default().bg(Color::Rgb(20, 20, 50));
    let mut lines: Vec<Line<'_>> = Vec::new();
    let phase_text = tui_state.status_message.as_deref().unwrap_or("Working...");

    for (idx, (agent_id, display)) in tui_state.agent_displays.iter().enumerate() {
        if idx > 0 {
            lines.push(Line::styled("", bg));
        }
        let spinner = display.spinner_char();
        let short_id = &agent_id.to_string()[..8];
        lines.push(Line::from(vec![
            Span::styled(format!(" {spinner} "), bg.fg(Color::Yellow)),
            Span::styled(
                format!("Agent {short_id}"),
                bg.fg(Color::Cyan).add_modifier(Modifier::BOLD),
            ),
            Span::styled(format!("  {phase_text}"), bg.fg(Color::DarkGray)),
        ]));

        render_agent_detail_line(&display.phase, area.width, bg, &mut lines);
    }

    let text = Text::from(lines);
    frame.render_widget(Paragraph::new(text).style(bg), area);
}

/// Append a streaming preview or phase detail line for one agent in the header.
fn render_agent_detail_line(
    phase: &crate::tui::AgentVisualPhase,
    width: u16,
    bg: Style,
    lines: &mut Vec<Line<'_>>,
) {
    use crate::tui::widgets::tool_status::truncate;

    match phase {
        crate::tui::AgentVisualPhase::Streaming { text, is_thinking } => {
            let label = if *is_thinking { "thinking" } else { "writing" };
            let preview = truncate(
                text.lines().next_back().unwrap_or(""),
                width.saturating_sub(8) as usize,
            );
            lines.push(Line::from(vec![
                Span::styled(format!("   [{label}] "), bg.fg(Color::DarkGray)),
                Span::styled(preview, bg.fg(Color::White)),
            ]));
        }
        crate::tui::AgentVisualPhase::ExecutingTools => {
            lines.push(Line::from(Span::styled(
                "   Running tool calls...",
                bg.fg(Color::DarkGray),
            )));
        }
        crate::tui::AgentVisualPhase::Preparing => {
            lines.push(Line::from(Span::styled(
                "   Preparing...",
                bg.fg(Color::DarkGray),
            )));
        }
    }
}

/// Render the tab bar: tab labels with active highlight.
fn render_tab_bar(frame: &mut Frame, area: Rect, tui_state: &TuiState) {
    let bg = Style::default().bg(Color::Rgb(30, 30, 80));
    let mut spans: Vec<Span> = Vec::new();

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

    // Pad the rest of the line with the tab bar background.
    let left_width: usize = spans.iter().map(Span::width).sum();
    let pad = (area.width as usize).saturating_sub(left_width);
    spans.push(Span::styled(" ".repeat(pad), bg));

    let line = Line::from(spans);
    frame.render_widget(Paragraph::new(line).style(bg), area);
}

/// Render the search bar: a single styled line between the tab bar and tab content.
///
/// Shows `/ query_text` on the left with a cursor, and match count + scope
/// indicator on the right. Yellow foreground distinguishes it visually.
fn render_search_bar(frame: &mut Frame, area: Rect, tui_state: &TuiState) {
    let Some(search) = &tui_state.search else {
        return;
    };

    let bg = Style::default().bg(Color::Rgb(40, 40, 20));
    let query_style = bg.fg(Color::Yellow);
    let dim = bg.fg(Color::DarkGray);

    // Left: "/ query_text" with cursor indicator.
    let cursor_char = "\u{2588}"; // █ block cursor
    let query_display = format!("/ {}{}", search.query_text, cursor_char);

    // Right: match count + scope.
    let scope_label = match search.scope {
        crate::tui::search::SearchScope::Tab => "Tab",
        crate::tui::search::SearchScope::Global => "Global",
    };
    let match_count = search.matching_ids.len();
    let right_text = format!("{match_count} matches  [{scope_label}]");

    let left_width = query_display.len();
    let right_width = right_text.len();
    let width = area.width as usize;
    let pad = width.saturating_sub(left_width + right_width + 1);

    let line = Line::from(vec![
        Span::styled(query_display, query_style),
        Span::styled(" ".repeat(pad), bg),
        Span::styled(right_text, dim),
    ]);
    frame.render_widget(Paragraph::new(line).style(bg), area);
}

/// Dispatch to the active tab's renderer.
fn render_tab_content(
    frame: &mut Frame,
    area: Rect,
    graph: &ConversationGraph,
    tui_state: &mut TuiState,
) {
    match tui_state.nav.active_tab {
        crate::tui::state::TopTab::Overview => {
            tabs::overview::render(frame, area, graph, tui_state);
        }
        crate::tui::state::TopTab::Graph => {
            tabs::graph::render(frame, area, graph, tui_state);
        }
        crate::tui::state::TopTab::System => {
            tabs::system::render(frame, area, graph, tui_state);
        }
    }
}

/// Status bar with branch/token info, context-aware shortcuts, and errors.
///
/// Combines the branch/token info (previously in the tab status bar) with
/// shortcut hints and error display.
fn draw_status_bar(frame: &mut Frame, area: Rect, graph: &ConversationGraph, tui_state: &TuiState) {
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

    // Right: branch + tokens + error.
    let right_text = build_right_status(graph, tui_state);

    let left_width: usize = spans.iter().map(Span::width).sum();
    let width = area.width as usize;
    let pad = width.saturating_sub(left_width + right_text.len());
    spans.push(Span::styled(" ".repeat(pad), bg));
    spans.push(Span::styled(right_text, bg.fg(Color::White)));

    let line = Line::from(spans);
    frame.render_widget(Paragraph::new(line).style(bg), area);
}

/// Build the right-aligned portion of the status bar: [branch] tokens error.
fn build_right_status(graph: &ConversationGraph, tui_state: &TuiState) -> String {
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

    let error_text = tui_state
        .error_message
        .as_ref()
        .map_or(String::new(), Clone::clone);

    let mut parts = vec![format!("[{branch}]")];
    if !token_text.is_empty() {
        parts.push(token_text);
    }
    if !error_text.is_empty() {
        parts.push(format!("ERR: {error_text}"));
    }
    parts.join("  ")
}

/// Build context-aware shortcut hints based on the current focus zone,
/// active tab, and (for the Graph tab) the explorer focus.
fn build_shortcuts(tui_state: &TuiState) -> Vec<(&'static str, &'static str)> {
    // Search mode shows its own shortcuts.
    if tui_state.search.is_some() {
        return vec![("Esc", "close"), ("Ctrl+G", "scope"), ("Ctrl+Q", "quit")];
    }

    match tui_state.nav.focus {
        FocusZone::ChatPanel => {
            if tui_state.pending_question_text.is_some() {
                vec![
                    ("Enter", "answer"),
                    ("Esc", "dismiss"),
                    ("Tab", "chat"),
                    ("Ctrl+Q", "quit"),
                ]
            } else {
                vec![("Enter", "send"), ("Tab", "chat"), ("Ctrl+Q", "quit")]
            }
        }
        FocusZone::TabContent => build_tab_content_shortcuts(tui_state),
    }
}

/// Build shortcut hints specific to `TabContent` focus, varying by active tab.
fn build_tab_content_shortcuts(tui_state: &TuiState) -> Vec<(&'static str, &'static str)> {
    use crate::tui::state::{ExplorerFocus, TopTab};

    match tui_state.nav.active_tab {
        TopTab::Overview | TopTab::System => {
            vec![
                ("Up/Dn", "nav"),
                ("/", "search"),
                ("Tab", "chat"),
                ("Ctrl+Q", "quit"),
            ]
        }
        TopTab::Graph => {
            let section = tui_state.nav.active_graph_section;
            let focus = tui_state
                .explorer
                .get(&section)
                .map_or(ExplorerFocus::Tree, |e| e.focus);
            match focus {
                ExplorerFocus::Tree => vec![
                    ("[/]", "section"),
                    ("Space", "toggle"),
                    ("Enter", "detail"),
                    ("/", "search"),
                    ("Tab", "chat"),
                    ("Ctrl+Q", "quit"),
                ],
                ExplorerFocus::Detail => vec![
                    ("Up/Dn", "edges"),
                    ("Enter", "follow"),
                    ("Esc", "back"),
                    ("Tab", "chat"),
                    ("Ctrl+Q", "quit"),
                ],
            }
        }
    }
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
