//! Tree widget for the Work tab.
//!
//! Renders `WorkItem` nodes as a tree using `SubtaskOf` edges for hierarchy.
//! Plans (top-level items) are expandable containers; tasks render indented.

use crate::graph::{ConversationGraph, Node, WorkItemKind, WorkItemStatus};
use ratatui::prelude::*;
use ratatui::widgets::{List, ListItem};
use std::collections::HashSet;
use uuid::Uuid;

/// Persistent state for the work tree widget.
#[derive(Debug, Default)]
pub struct WorkTreeState {
    /// UUIDs of expanded items (collapsed by default).
    pub expanded: HashSet<Uuid>,
}

/// Render the Work tab as a tree of plans and tasks.
pub fn render(frame: &mut Frame, area: Rect, graph: &ConversationGraph, state: &WorkTreeState) {
    let mut items = Vec::new();
    let plans = collect_plans(graph);

    for plan in &plans {
        render_plan_item(graph, plan, state, 0, &mut items);
    }

    if items.is_empty() {
        items.push(ListItem::new(Line::from(Span::styled(
            "No plans yet. Use /plan <description> to create one.",
            Style::default().fg(Color::DarkGray),
        ))));
    }

    let list = List::new(items);
    frame.render_widget(list, area);
}

/// Collect all Plan-kind `WorkItem` nodes that have no parent (root plans).
fn collect_plans(graph: &ConversationGraph) -> Vec<Uuid> {
    graph
        .nodes_by(|n| {
            matches!(
                n,
                Node::WorkItem {
                    kind: WorkItemKind::Plan,
                    ..
                }
            )
        })
        .into_iter()
        .filter(|n| graph.parent_of(n.id()).is_none())
        .map(Node::id)
        .collect()
}

/// Render a single work item and its children recursively.
fn render_plan_item(
    graph: &ConversationGraph,
    node_id: &Uuid,
    state: &WorkTreeState,
    depth: usize,
    items: &mut Vec<ListItem<'static>>,
) {
    let Some(Node::WorkItem {
        id,
        title,
        kind,
        status,
        ..
    }) = graph.node(*node_id)
    else {
        return;
    };

    let indent = "  ".repeat(depth);
    let (status_marker, color) = status_style(status);
    let has_children = !graph.children_of(*id).is_empty();
    let is_expanded = state.expanded.contains(id);

    // Show dependencies for Plan-kind items.
    let dep_suffix = if *kind == WorkItemKind::Plan {
        let deps = graph.dependencies_of(*id);
        if deps.is_empty() {
            String::new()
        } else {
            let dep_names: Vec<String> = deps
                .iter()
                .filter_map(|dep_id| match graph.node(*dep_id) {
                    Some(Node::WorkItem { title, .. }) => Some(title.clone()),
                    _ => None,
                })
                .collect();
            format!(" (depends on: {})", dep_names.join(", "))
        }
    } else {
        String::new()
    };

    let expand_marker = if has_children {
        if is_expanded {
            "v "
        } else {
            "> "
        }
    } else {
        "  "
    };

    items.push(ListItem::new(Line::from(vec![
        Span::raw(format!("{indent}{expand_marker}")),
        Span::styled(format!("[{status_marker}] "), Style::default().fg(color)),
        Span::styled(title.clone(), Style::default().fg(Color::White)),
        Span::styled(dep_suffix, Style::default().fg(Color::DarkGray)),
    ])));

    // Render children if expanded.
    if is_expanded {
        let children = graph.children_of(*id);
        for child_id in &children {
            render_plan_item(graph, child_id, state, depth + 1, items);
        }
    }
}

/// Map status to a display marker and color.
fn status_style(status: &WorkItemStatus) -> (&'static str, Color) {
    match status {
        WorkItemStatus::Todo => (" ", Color::Yellow),
        WorkItemStatus::Active => ("*", Color::Cyan),
        WorkItemStatus::Done => ("v", Color::Green),
    }
}
