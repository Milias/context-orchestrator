//! Graph explorer tab: collapsible sections with tree + detail layout.
//!
//! The Graph tab shows four sections (Work, QA, Execution, Context).
//! Only one section is expanded at a time, showing a tree panel on the
//! left and a detail panel on the right. Collapsed sections render as
//! one-line summary headers. Sub-modules handle tree drawing, the work
//! tree, and the node detail panel.

mod context;
mod detail;
mod execution;
mod qa;
pub mod tree_lines;
mod work;

use crate::graph::{ConversationGraph, Node};
use crate::tui::state::GraphSection;
use crate::tui::TuiState;

use ratatui::prelude::*;
use ratatui::widgets::Paragraph;

/// Arrow prefix for the expanded (active) section header.
const EXPANDED_ARROW: &str = "\u{25bc} "; // ▼
/// Arrow prefix for collapsed (inactive) section headers.
const COLLAPSED_ARROW: &str = "\u{25b6} "; // ▶

/// Render the Graph tab with collapsible sections.
///
/// Vertical layout: one row per section. The active section gets flexible
/// space and renders as a tree (60%) + detail (40%) horizontal split.
/// Collapsed sections render as single-line summary headers.
pub fn render(
    frame: &mut Frame,
    area: Rect,
    graph: &ConversationGraph,
    tui_state: &mut TuiState,
) {
    let active = tui_state.nav.active_graph_section;
    let sections = GraphSection::all();

    // Build vertical constraints: 1 line per collapsed section, flexible for active.
    let constraints: Vec<Constraint> = sections
        .iter()
        .map(|s| {
            if *s == active {
                Constraint::Min(3)
            } else {
                Constraint::Length(1)
            }
        })
        .collect();

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(area);

    for (i, &section) in sections.iter().enumerate() {
        if section == active {
            render_expanded_section(frame, rows[i], section, graph, tui_state);
        } else {
            render_collapsed_header(frame, rows[i], section, graph);
        }
    }
}

/// Render an expanded section: header line + tree (60%) | detail (40%).
fn render_expanded_section(
    frame: &mut Frame,
    area: Rect,
    section: GraphSection,
    graph: &ConversationGraph,
    tui_state: &mut TuiState,
) {
    // Split: 1-line header + content below.
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(1)])
        .split(area);

    render_active_header(frame, chunks[0], section);

    // Horizontal split: tree (60%) | detail (40%).
    let panels = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
        .split(chunks[1]);

    // Store panel rects for mouse hit-testing.
    tui_state.panel_rects.tree = panels[0];
    tui_state.panel_rects.detail = panels[1];

    match section {
        GraphSection::Work => {
            let selected_id = work::render(frame, panels[0], graph, tui_state);
            detail::render(frame, panels[1], graph, selected_id, tui_state);
        }
        GraphSection::QA => {
            let selected_id = qa::render(frame, panels[0], graph, tui_state);
            detail::render(frame, panels[1], graph, selected_id, tui_state);
        }
        GraphSection::Execution => {
            let selected_id = execution::render(frame, panels[0], graph, tui_state);
            detail::render(frame, panels[1], graph, selected_id, tui_state);
        }
        GraphSection::Context => {
            let selected_id = context::render(frame, panels[0], graph, tui_state);
            detail::render(frame, panels[1], graph, selected_id, tui_state);
        }
    }
}

/// Render the active section header: `▼ [Work]`.
fn render_active_header(frame: &mut Frame, area: Rect, section: GraphSection) {
    let style = Style::default()
        .fg(Color::Cyan)
        .add_modifier(Modifier::BOLD);
    let line = Line::from(vec![
        Span::styled(EXPANDED_ARROW, style),
        Span::styled(format!("[{}]", section.label()), style),
    ]);
    frame.render_widget(Paragraph::new(line), area);
}

/// Render a collapsed section header with summary counts.
///
/// Format: `▶ [Work] (3 plans, 12 tasks)`.
fn render_collapsed_header(
    frame: &mut Frame,
    area: Rect,
    section: GraphSection,
    graph: &ConversationGraph,
) {
    let summary = section_summary(section, graph);
    let dim = Style::default().fg(Color::DarkGray);
    let line = Line::from(vec![
        Span::styled(COLLAPSED_ARROW, dim),
        Span::styled(format!("[{}]", section.label()), dim),
        Span::styled(format!(" ({summary})"), Style::default().fg(Color::Rgb(80, 80, 100))),
    ]);
    frame.render_widget(Paragraph::new(line), area);
}

/// Compute a summary string for a collapsed section header.
///
/// Returns something like `"3 plans, 12 tasks"` for Work or
/// `"5 questions"` for QA.
fn section_summary(section: GraphSection, graph: &ConversationGraph) -> String {
    match section {
        GraphSection::Work => {
            let mut plans = 0u32;
            let mut tasks = 0u32;
            for node in graph.nodes_by(|n| matches!(n, Node::WorkItem { .. })) {
                if let Node::WorkItem { kind, .. } = node {
                    match kind {
                        crate::graph::WorkItemKind::Plan => plans += 1,
                        crate::graph::WorkItemKind::Task => tasks += 1,
                    }
                }
            }
            format!("{plans} plans, {tasks} tasks")
        }
        GraphSection::QA => {
            let count = graph
                .nodes_by(|n| matches!(n, Node::Question { .. }))
                .len();
            format!("{count} questions")
        }
        GraphSection::Execution => {
            let count = graph
                .nodes_by(|n| matches!(n, Node::ToolCall { .. }))
                .len();
            format!("{count} tool calls")
        }
        GraphSection::Context => {
            let count = graph
                .nodes_by(|n| matches!(n, Node::ContextBuildingRequest { .. }))
                .len();
            format!("{count} builds")
        }
    }
}
