//! Build a plan context section for injection into the system prompt.
//!
//! Queries all `WorkItem` nodes from the graph, resolves `SubtaskOf` and
//! `DependsOn` edges, and renders a tree of active plans for the LLM.

use crate::graph::{ConversationGraph, Node, WorkItemKind, WorkItemStatus};
use uuid::Uuid;

/// Build the plan section for the system prompt.
/// Returns `None` if no plans exist (avoids injecting an empty section).
pub fn build_plan_section(graph: &ConversationGraph) -> Option<String> {
    let plans: Vec<&Node> = graph.nodes_by(|n| {
        matches!(
            n,
            Node::WorkItem {
                kind: WorkItemKind::Plan,
                status,
                ..
            } if *status != WorkItemStatus::Done
        )
    });

    if plans.is_empty() {
        return None;
    }

    let mut lines = vec!["## Active Plans".to_string()];

    for plan in &plans {
        let Node::WorkItem {
            id, title, status, ..
        } = plan
        else {
            continue;
        };

        let status_label = status_marker(*status);
        lines.push(format!("Plan: \"{title}\" [{status_label}] (id: {id})"));

        // Show dependencies.
        let deps = graph.dependencies_of(*id);
        for dep_id in &deps {
            if let Some(Node::WorkItem {
                title: dep_title, ..
            }) = graph.node(*dep_id)
            {
                lines.push(format!("  depends on: \"{dep_title}\" (id: {dep_id})"));
            }
        }

        // Show child tasks.
        render_children(graph, *id, 1, &mut lines);
    }

    Some(lines.join("\n"))
}

/// Recursively render children of a work item at the given indentation depth.
fn render_children(
    graph: &ConversationGraph,
    parent_id: Uuid,
    depth: usize,
    lines: &mut Vec<String>,
) {
    let children = graph.children_of(parent_id);
    let indent = "  ".repeat(depth);
    for child_id in &children {
        if let Some(Node::WorkItem {
            id, title, status, ..
        }) = graph.node(*child_id)
        {
            let marker = status_marker(*status);
            lines.push(format!("{indent}- [{marker}] {title} (id: {id})"));
            render_children(graph, *id, depth + 1, lines);
        }
    }
}

/// Map `WorkItemStatus` to a compact display marker.
fn status_marker(status: WorkItemStatus) -> &'static str {
    match status {
        WorkItemStatus::Todo => "todo",
        WorkItemStatus::Active => "active",
        WorkItemStatus::Done => "done",
    }
}

#[cfg(test)]
#[path = "context_tests.rs"]
mod tests;
