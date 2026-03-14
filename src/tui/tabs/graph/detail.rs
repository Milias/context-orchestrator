//! Detail panel renderer for the Graph tab.
//!
//! Renders the right side of the tree+detail split. Shows a selected
//! node's header (type badge, UUID, status), full content, edges grouped
//! by semantic category, and a breadcrumb trail when navigating via edges.

use std::collections::BTreeMap;

use uuid::Uuid;

use crate::graph::{ConversationGraph, EdgeDirection, EdgeKind};
use crate::tui::tabs::edge_inspector::EdgeInspector;
use crate::tui::widgets::tool_status::truncate;
use crate::tui::TuiState;

use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Paragraph};

/// Maximum content preview lines before truncation.
const MAX_CONTENT_LINES: usize = 12;

/// Short UUID: first 8 hex characters.
fn short_uuid(id: Uuid) -> String {
    id.to_string()[..8].to_string()
}

/// Render the detail panel for a selected graph node.
///
/// Shows: breadcrumb trail, header (type badge, UUID, status),
/// content (full text), and edges grouped by semantic category.
/// Falls back to a placeholder when no node is selected.
pub fn render(
    frame: &mut Frame,
    area: Rect,
    graph: &ConversationGraph,
    node_id: Option<Uuid>,
    tui_state: &TuiState,
) {
    let block = Block::default().title("Detail").borders(Borders::ALL);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.height == 0 || inner.width < 8 {
        return;
    }

    let Some(id) = node_id else {
        render_empty(frame, inner);
        return;
    };

    let Some(node) = graph.node(id) else {
        render_empty(frame, inner);
        return;
    };

    let width = inner.width as usize;
    let max_lines = inner.height as usize;
    let mut lines: Vec<Line<'_>> = Vec::new();

    // Breadcrumb trail.
    render_breadcrumb(&tui_state.edge_inspector, graph, width, &mut lines);

    // Header: [BADGE] short_uuid  status  created_at.
    render_header(node, &mut lines);

    // Blank separator.
    lines.push(Line::raw(""));

    // Content section.
    render_content(node.content(), width, max_lines, &mut lines);

    // Blank separator before edges.
    lines.push(Line::raw(""));

    // Edges section.
    render_edges(graph, id, &tui_state.edge_inspector, width, &mut lines);

    // Truncate to available height.
    lines.truncate(max_lines);
    frame.render_widget(Paragraph::new(Text::from(lines)), inner);
}

/// Render the "(no selection)" placeholder.
fn render_empty(frame: &mut Frame, area: Rect) {
    let p = Paragraph::new(Span::styled(
        "(select an item)",
        Style::default().fg(Color::DarkGray),
    ));
    frame.render_widget(p, area);
}

/// Render the breadcrumb trail at the top of the detail panel.
///
/// Format: `trail: NodeA > NodeB > here`
/// Only shown when the inspector trail is non-empty.
fn render_breadcrumb(
    inspector: &EdgeInspector,
    graph: &ConversationGraph,
    width: usize,
    lines: &mut Vec<Line<'_>>,
) {
    if inspector.trail.is_empty() {
        return;
    }

    let dim = Style::default().fg(Color::DarkGray);
    let mut spans = vec![Span::styled("trail: ", dim)];

    for (i, crumb) in inspector.trail.iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled(" > ", dim));
        }
        let label = graph.node(crumb.node_id).map_or_else(
            || short_uuid(crumb.node_id),
            |n| truncate(n.content().lines().next().unwrap_or("?"), 15),
        );
        spans.push(Span::styled(label, Style::default().fg(Color::Cyan)));
    }
    spans.push(Span::styled(" > here", dim));

    // If the trail spans are too wide, fall back to a depth count.
    let total_width: usize = spans.iter().map(Span::width).sum();
    if total_width <= width {
        lines.push(Line::from(spans));
    } else {
        lines.push(Line::from(Span::styled(
            format!("trail: {} steps deep", inspector.trail.len()),
            dim,
        )));
    }
}

/// Render the header: `[BADGE] short_uuid  [status]  HH:MM:SS`.
fn render_header(node: &crate::graph::Node, lines: &mut Vec<Line<'_>>) {
    let badge = node.type_badge();
    let uuid_short = short_uuid(node.id());
    let timestamp = node.created_at().format("%H:%M:%S").to_string();

    let mut spans = vec![
        Span::styled(
            format!("[{badge}]"),
            Style::default().fg(Color::Yellow).bold(),
        ),
        Span::styled(
            format!(" {uuid_short}"),
            Style::default().fg(Color::DarkGray),
        ),
    ];

    if let Some(status) = node.status_label() {
        spans.push(Span::styled(
            format!("  [{status}]"),
            Style::default().fg(Color::Cyan),
        ));
    }

    spans.push(Span::styled(
        format!("  {timestamp}"),
        Style::default().fg(Color::DarkGray),
    ));

    lines.push(Line::from(spans));

    // Second line: first line of content as a preview.
    let first_line = node.content().lines().next().unwrap_or("");
    if !first_line.is_empty() {
        lines.push(Line::from(Span::styled(
            truncate(first_line, 60),
            Style::default().fg(Color::White),
        )));
    }
}

/// Render the full content section.
///
/// Shows up to [`MAX_CONTENT_LINES`] of the content text, truncated at `width`.
/// Reserves some lines for the edges section below.
fn render_content(
    content: &str,
    width: usize,
    budget_lines: usize,
    lines: &mut Vec<Line<'_>>,
) {
    if content.is_empty() {
        lines.push(Line::from(Span::styled(
            "(empty)",
            Style::default().fg(Color::DarkGray),
        )));
        return;
    }

    // Reserve room for edges section.
    let max_here = budget_lines.saturating_sub(8).clamp(3, MAX_CONTENT_LINES);
    let total_content_lines = content.lines().count();

    for (i, line) in content.lines().enumerate() {
        if i >= max_here {
            let remaining = total_content_lines - i;
            lines.push(Line::from(Span::styled(
                format!("[... {remaining} more lines]"),
                Style::default().fg(Color::DarkGray),
            )));
            break;
        }
        lines.push(Line::from(Span::styled(
            truncate(line, width),
            Style::default().fg(Color::White),
        )));
    }
}

/// Render edges grouped by [`EdgeGroup`].
///
/// Each group header is bold cyan, followed by edges as
/// `direction label -> target_summary (short_uuid)`.
/// The currently selected edge (from the inspector) is highlighted.
fn render_edges(
    graph: &ConversationGraph,
    node_id: Uuid,
    inspector: &EdgeInspector,
    width: usize,
    lines: &mut Vec<Line<'_>>,
) {
    let raw_edges = graph.edges_of(node_id);
    if raw_edges.is_empty() {
        lines.push(Line::from(Span::styled(
            "(no edges)",
            Style::default().fg(Color::DarkGray),
        )));
        return;
    }

    // Group edges by EdgeGroup using BTreeMap keyed on the group label
    // for stable display ordering.
    let mut groups: BTreeMap<&'static str, Vec<(EdgeDirection, EdgeKind, Uuid)>> = BTreeMap::new();
    for (dir, kind, other_id) in &raw_edges {
        groups
            .entry(kind.group().label())
            .or_default()
            .push((*dir, *kind, *other_id));
    }

    let mut edge_index = 0usize;
    for (group_label, edges) in &groups {
        // Group header.
        lines.push(Line::from(Span::styled(
            format!("--- {group_label} ---"),
            Style::default().fg(Color::Cyan).bold(),
        )));

        for (dir, kind, other_id) in edges {
            let is_selected = edge_index == inspector.selected_edge;
            let dir_arrow = match dir {
                EdgeDirection::Outgoing => "\u{2192}", // ->
                EdgeDirection::Incoming => "\u{2190}", // <-
            };
            let label = kind.display_label();
            let target_summary = graph.node(*other_id).map_or_else(
                || short_uuid(*other_id),
                |n| truncate(n.content().lines().next().unwrap_or("?"), 25),
            );
            let target_short = short_uuid(*other_id);

            let edge_text = format!("{dir_arrow} {label} {target_summary} ({target_short})");
            let line_budget = width.saturating_sub(4);
            let display_text = truncate(&edge_text, line_budget);

            let style = if is_selected {
                Style::default()
                    .fg(Color::White)
                    .bg(Color::Rgb(40, 40, 60))
                    .bold()
            } else {
                Style::default().fg(Color::DarkGray)
            };

            let prefix = if is_selected { " > " } else { "   " };
            lines.push(Line::from(vec![
                Span::styled(prefix, style),
                Span::styled(display_text, style),
            ]));

            edge_index += 1;
        }
    }
}
