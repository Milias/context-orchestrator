//! Work tree utilities: building and rendering plan/task trees.
//!
//! Provides building blocks for displaying work items as a tree.
//! Plans are top-level, tasks are nested under their parent via
//! `SubtaskOf` edges. Used by the overview tab.

use crate::graph::node::{WorkItemKind, WorkItemStatus};
use crate::graph::{ConversationGraph, Node};
use crate::tui::widgets::tool_status::truncate;

use ratatui::prelude::*;

/// A flattened work item with its children for tree rendering.
pub(super) struct WorkTreeItem {
    title: String,
    kind: WorkItemKind,
    pub(super) status: WorkItemStatus,
    deps: Vec<String>,
    children: Vec<WorkTreeItem>,
}

/// Build the work tree from graph nodes.
/// Plans are roots; tasks nest under their parent via `SubtaskOf`.
pub(super) fn build_work_tree(graph: &ConversationGraph) -> Vec<WorkTreeItem> {
    // Collect all work items.
    let work_items: Vec<&Node> = graph.nodes_by(|n| matches!(n, Node::WorkItem { .. }));

    // Find root items (no SubtaskOf parent).
    let mut roots = Vec::new();
    for node in &work_items {
        let id = node.id();
        if graph.parent_of(id).is_none() {
            roots.push(build_item(graph, node));
        }
    }

    // Sort: plans first, then by creation time.
    roots.sort_by(|a, b| {
        let kind_order = |k: &WorkItemKind| match k {
            WorkItemKind::Plan => 0,
            WorkItemKind::Task => 1,
        };
        kind_order(&a.kind)
            .cmp(&kind_order(&b.kind))
            .then(a.title.cmp(&b.title))
    });

    roots
}

/// Recursively build a work tree item from a graph node.
fn build_item(graph: &ConversationGraph, node: &Node) -> WorkTreeItem {
    let Node::WorkItem {
        id,
        title,
        kind,
        status,
        ..
    } = node
    else {
        unreachable!("build_item called on non-WorkItem");
    };

    let deps: Vec<String> = graph
        .dependencies_of(*id)
        .iter()
        .filter_map(|dep_id| graph.node(*dep_id).map(|n| n.content().to_string()))
        .collect();

    let child_ids = graph.children_of(*id);
    let mut children: Vec<WorkTreeItem> = child_ids
        .iter()
        .filter_map(|cid| graph.node(*cid))
        .map(|n| build_item(graph, n))
        .collect();
    children.sort_by(|a, b| a.title.cmp(&b.title));

    WorkTreeItem {
        title: title.clone(),
        kind: *kind,
        status: *status,
        deps,
        children,
    }
}

/// Render a single item and its children recursively.
/// `selected_idx` highlights the item at that flat index.
pub(super) fn render_item(
    lines: &mut Vec<Line<'static>>,
    item: &WorkTreeItem,
    depth: usize,
    width: usize,
    max_lines: usize,
    selected_idx: usize,
) {
    if lines.len() >= max_lines {
        return;
    }

    let is_selected = lines.len() == selected_idx;
    let indent = "  ".repeat(depth);
    let (icon, icon_color) = status_icon(item.status);
    let kind_prefix = match item.kind {
        WorkItemKind::Plan => "v ",
        WorkItemKind::Task => "",
    };

    let title_budget = width.saturating_sub(indent.len() + 4 + kind_prefix.len());
    let title = truncate(&item.title, title_budget);

    let base_color = match item.kind {
        WorkItemKind::Plan => Color::Yellow,
        WorkItemKind::Task => Color::White,
    };
    let title_style = if is_selected {
        Style::default()
            .fg(base_color)
            .bg(Color::Rgb(40, 40, 60))
            .add_modifier(Modifier::BOLD)
    } else if matches!(item.kind, WorkItemKind::Plan) {
        Style::default().fg(base_color).bold()
    } else {
        Style::default().fg(base_color)
    };

    let mut spans = vec![
        Span::raw(indent.clone()),
        Span::styled(format!("{icon} "), Style::default().fg(icon_color)),
        Span::styled(kind_prefix, Style::default().fg(Color::DarkGray)),
        Span::styled(title, title_style),
    ];
    if is_selected {
        spans.insert(0, Span::styled("→ ", Style::default().fg(Color::Cyan)));
    }

    // Show dependency annotations.
    if !item.deps.is_empty() {
        let dep_names: String = item
            .deps
            .iter()
            .map(|d| truncate(d, 20))
            .collect::<Vec<_>>()
            .join(", ");
        spans.push(Span::styled(
            "  (depends on: ",
            Style::default().fg(Color::DarkGray),
        ));
        spans.push(Span::styled(dep_names, Style::default().fg(Color::Magenta)));
        spans.push(Span::styled(")", Style::default().fg(Color::DarkGray)));
    }

    lines.push(Line::from(spans));

    // Render children.
    for child in &item.children {
        render_item(lines, child, depth + 1, width, max_lines, selected_idx);
    }
}

/// Status icon and color for a work item.
pub(super) fn status_icon(status: WorkItemStatus) -> (&'static str, Color) {
    match status {
        WorkItemStatus::Todo => ("[ ]", Color::DarkGray),
        WorkItemStatus::Active => ("[*]", Color::Yellow),
        WorkItemStatus::Done => ("[v]", Color::Green),
    }
}
