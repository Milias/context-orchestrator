//! Work tab: plan/task tree with dependency tracking.
//!
//! Displays all work items (plans and tasks) as a tree. Plans are
//! top-level, tasks are nested under their parent via `SubtaskOf` edges.
//! Dependency annotations show which items block others.

use crate::graph::node::{WorkItemKind, WorkItemStatus};
use crate::graph::{ConversationGraph, Node};
use crate::tui::widgets::tool_status::truncate;
use crate::tui::TuiState;

use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Paragraph};

/// Render the Work tab content into the given area.
pub fn render(frame: &mut Frame, area: Rect, graph: &ConversationGraph, tui_state: &mut TuiState) {
    let block = Block::default().title("Work").borders(Borders::ALL);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.height == 0 || inner.width < 8 {
        return;
    }

    let tree = build_work_tree(graph);

    if tree.is_empty() {
        let empty = Paragraph::new(Span::styled(
            "(no plans or tasks)",
            Style::default().fg(Color::DarkGray),
        ));
        frame.render_widget(empty, inner);
        return;
    }

    let width = inner.width as usize;
    let max_lines = inner.height as usize;
    let mut lines: Vec<Line<'_>> = Vec::new();

    for item in &tree {
        if lines.len() >= max_lines {
            break;
        }
        render_item(
            &mut lines,
            item,
            0,
            width,
            max_lines,
            tui_state.work_selected,
        );
    }

    // Publish visible count so input handler can clamp selection.
    tui_state.work_visible_count = lines.len();

    frame.render_widget(Paragraph::new(Text::from(lines)), inner);
}

/// A flattened work item with its children for tree rendering.
struct WorkTreeItem {
    title: String,
    kind: WorkItemKind,
    status: WorkItemStatus,
    deps: Vec<String>,
    children: Vec<WorkTreeItem>,
}

/// Build the work tree from graph nodes.
/// Plans are roots; tasks nest under their parent via `SubtaskOf`.
fn build_work_tree(graph: &ConversationGraph) -> Vec<WorkTreeItem> {
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
fn render_item(
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

    let title_style = if is_selected {
        Style::default()
            .fg(Color::White)
            .bg(Color::Rgb(40, 40, 60))
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::White)
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
            format!("  (depends on: {dep_names})"),
            Style::default().fg(Color::DarkGray),
        ));
    }

    lines.push(Line::from(spans));

    // Render children.
    for child in &item.children {
        render_item(lines, child, depth + 1, width, max_lines, selected_idx);
    }
}

/// Status icon and color for a work item.
fn status_icon(status: WorkItemStatus) -> (&'static str, Color) {
    match status {
        WorkItemStatus::Todo => ("[ ]", Color::DarkGray),
        WorkItemStatus::Active => ("[*]", Color::Yellow),
        WorkItemStatus::Done => ("[v]", Color::Green),
    }
}
