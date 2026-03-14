//! System tab: stacked collapsible sections showing activity, files, errors,
//! tools, and stats.
//!
//! Layout:
//! ```text
//! ┌─ Activity ─────────────────────────────────────┐
//! │  14:32:08 ✓ read_file src/main.rs       0.3s   │
//! │  14:32:01 A [Assistant] Let me read...         │
//! ├─ Files ────────────────────────────────────────┤
//! │  src/                                          │
//! │    tui/                                        │
//! │      mod.rs [Modified]                         │
//! ├─ Errors (0) ───────────────────────────────────┤
//! ├─ Tools ────────────────────────────────────────┤
//! │  plan         Create a plan...                 │
//! ├─ Stats ────────────────────────────────────────┤
//! │  Tokens: 45.3k in / 12.1k out  Msgs: 47       │
//! └────────────────────────────────────────────────┘
//! ```
//!
//! The Activity section gets flexible height (`Min(5)`) since it is the
//! largest. Other sections size to their content. Empty sections (e.g.
//! Errors with 0 entries) collapse to a single header line.

mod activity;
mod files;

use crate::graph::ConversationGraph;
use crate::tui::widgets::{stats_panel, tools_panel};
use crate::tui::TuiState;

use ratatui::prelude::*;

/// Render the System tab with stacked collapsible sections.
///
/// Sections: Activity, Files, Errors, Tools, Stats. Each section has a
/// bordered header. Empty sections collapse to their header only (2 lines
/// for borders). Activity gets the remaining flexible space.
pub fn render(frame: &mut Frame, area: Rect, graph: &ConversationGraph, tui_state: &mut TuiState) {
    let files_h = files::files_section_height(graph);
    let errors_h = errors_section_height(graph);
    let tools_h = tools_panel::tools_panel_height();
    let stats_h: u16 = 9;

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(5), // Activity (flexible)
            Constraint::Length(files_h),
            Constraint::Length(errors_h),
            Constraint::Length(tools_h),
            Constraint::Length(stats_h),
        ])
        .split(area);

    activity::render_activity(frame, rows[0], graph, tui_state);
    files::render_files(frame, rows[1], graph);
    render_errors(frame, rows[2], graph);
    tools_panel::render(frame, rows[3]);
    stats_panel::render(frame, rows[4], graph, tui_state);
}

// ── Errors section ──────────────────────────────────────────────────

use crate::graph::Node;
use ratatui::widgets::{Block, Borders, Paragraph};

/// Compute the errors section height.
///
/// Returns 2 (collapsed header with borders) when no errors exist,
/// or borders + error count (capped at 8 lines).
fn errors_section_height(graph: &ConversationGraph) -> u16 {
    let count = graph.nodes_by(|n| matches!(n, Node::ApiError { .. })).len();
    if count == 0 {
        return 2; // collapsed: just the header borders
    }
    let n = u16::try_from(count).unwrap_or(u16::MAX);
    n.saturating_add(2).min(10) // borders + cap
}

/// Render the Errors section: `ApiError` nodes sorted newest first.
///
/// When no errors exist, displays a collapsed header `Errors (0)`.
/// Otherwise shows each error as `! "message" [HH:MM:SS]`.
fn render_errors(frame: &mut Frame, area: Rect, graph: &ConversationGraph) {
    let mut errors: Vec<&Node> = graph.nodes_by(|n| matches!(n, Node::ApiError { .. }));
    errors.sort_by_key(|n| std::cmp::Reverse(n.created_at()));

    let title = if errors.is_empty() {
        "Errors (0)".to_string()
    } else {
        format!("Errors ({})", errors.len())
    };

    let block = Block::default().title(title).borders(Borders::ALL);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.height == 0 || inner.width < 8 || errors.is_empty() {
        return;
    }

    let width = inner.width as usize;
    let max_rows = inner.height as usize;

    let lines: Vec<Line<'_>> = errors
        .iter()
        .take(max_rows)
        .map(|node| render_error_line(node, width))
        .collect();

    frame.render_widget(Paragraph::new(Text::from(lines)), inner);
}

/// Render a single error line: `! "message" [HH:MM:SS]`.
fn render_error_line<'a>(node: &Node, width: usize) -> Line<'a> {
    let Node::ApiError {
        message,
        created_at,
        ..
    } = node
    else {
        return Line::raw("");
    };

    let time = created_at.format("%H:%M:%S").to_string();
    // Fixed: "! " (2) + " [HH:MM:SS]" (11).
    let fixed = 2 + 1 + time.len() + 2;
    let msg_budget = width.saturating_sub(fixed);
    let preview = crate::tui::widgets::tool_status::truncate(message, msg_budget);

    let dim = Style::default().fg(Color::DarkGray);

    Line::from(vec![
        Span::styled("! ", Style::default().fg(Color::Red)),
        Span::styled(preview, Style::default().fg(Color::White)),
        Span::styled(format!(" [{time}]"), dim),
    ])
}
