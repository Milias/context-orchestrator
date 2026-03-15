//! System tab: horizontal split layout showing activity alongside files,
//! errors, and tools.
//!
//! Layout:
//! ```text
//! ┌─ Activity ──────────┬─ Files ──────────────────┐
//! │  14:32:08 ✓ read_f  │  src/                    │
//! │  14:32:01 A [Asst]  │    tui/mod.rs [Modified] │
//! │                     ├─ Errors (0) ─────────────┤
//! │                     ├─ Tools ──────────────────┤
//! │                     │  plan    Create a plan…  │
//! └─────────────────────┴──────────────────────────┘
//! ```
//!
//! Left column (~35%): Activity panel (full height).
//! Right column (~65%): Files, Errors, Tools stacked vertically.
//! The Tools section gets flexible height via `Min(5)`.
//! Empty sections (e.g. Errors with 0 entries) collapse to a single header.

mod activity;
mod files;

use crate::graph::ConversationGraph;
use crate::tui::widgets::tools_panel;
use crate::tui::TuiState;

use ratatui::prelude::*;

/// Render the System tab with a horizontal split layout.
///
/// Left column (~35%): Activity panel (full height).
/// Right column (~65%): Files, Errors, Tools stacked vertically.
///
/// Each section has a bordered header. Empty sections collapse to their
/// header only (2 lines for borders).
pub fn render(frame: &mut Frame, area: Rect, graph: &ConversationGraph, tui_state: &mut TuiState) {
    let files_h = files::files_section_height(graph);
    let errors_h = errors_section_height(graph);

    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(35), Constraint::Percentage(65)])
        .split(area);

    // Left: Activity (full height). Track rect for mouse scroll.
    tui_state.panel_rects.activity = cols[0];
    activity::render_activity(frame, cols[0], graph, tui_state);

    // Right: Files + Errors + Tools stacked vertically.
    let right_stack = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(files_h),
            Constraint::Length(errors_h),
            Constraint::Min(5), // Tools gets remaining space.
        ])
        .split(cols[1]);

    files::render_files(frame, right_stack[0], graph);
    render_errors(frame, right_stack[1], graph);
    tools_panel::render(frame, right_stack[2]);
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
