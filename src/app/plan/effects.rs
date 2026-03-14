//! Side-effects for plan and task management tools.
//!
//! These run in `handle_tool_call_completed` after tool execution. They mutate
//! the graph only and return enriched tool result content with UUIDs.
//! TUI notifications flow through `GraphEvent`, not direct calls.

use crate::graph::event::GraphEvent;
use crate::graph::tool::result::ToolResultContent;
use crate::graph::tool::types::ToolCallArguments;
use crate::graph::{ConversationGraph, EdgeKind, Node, WorkItemKind, WorkItemStatus};

use chrono::Utc;
use uuid::Uuid;

/// Apply plan-tool side-effects for a completed tool call.
/// Returns enriched `ToolResultContent` if the tool is plan-related, `None` otherwise.
/// Only mutates the graph — TUI notifications flow through `GraphEvent`.
pub fn apply(graph: &mut ConversationGraph, tool_call_id: Uuid) -> Option<ToolResultContent> {
    let (arguments, parent_message_id) = match graph.node(tool_call_id)? {
        Node::ToolCall {
            arguments,
            parent_message_id,
            ..
        } => (arguments.clone(), *parent_message_id),
        _ => return None,
    };

    match &arguments {
        ToolCallArguments::Plan { title, description } => Some(apply_plan(
            graph,
            title,
            description.as_deref(),
            parent_message_id,
        )),
        ToolCallArguments::AddTask {
            parent_id,
            title,
            description,
        } => Some(apply_add_task(
            graph,
            *parent_id,
            title,
            description.as_deref(),
        )),
        ToolCallArguments::UpdateWorkItem {
            id,
            status,
            description,
        } => Some(apply_update_work_item(
            graph,
            *id,
            status.as_ref(),
            description.as_deref(),
        )),
        ToolCallArguments::AddDependency { from_id, to_id } => {
            Some(apply_add_dependency(graph, *from_id, *to_id))
        }
        _ => None,
    }
}

/// Create a `Plan`-kind `WorkItem`, link to parent message, return UUID in content.
fn apply_plan(
    graph: &mut ConversationGraph,
    title: &str,
    description: Option<&str>,
    parent_message_id: Uuid,
) -> ToolResultContent {
    let wi_id = Uuid::new_v4();
    let work_item = Node::WorkItem {
        id: wi_id,
        kind: WorkItemKind::Plan,
        title: title.to_string(),
        status: WorkItemStatus::Todo,
        description: description.map(String::from),
        created_at: Utc::now(),
    };
    graph.add_node(work_item);
    graph.emit(GraphEvent::WorkItemAdded {
        node_id: wi_id,
        kind: WorkItemKind::Plan,
    });
    let _ = graph.add_edge(wi_id, parent_message_id, EdgeKind::RelevantTo);
    ToolResultContent::text(format!(
        "Created plan '{title}' (id: {wi_id}). Use add_task to decompose it into steps."
    ))
}

/// Create a `Task`-kind `WorkItem` under a parent, link via `SubtaskOf`.
fn apply_add_task(
    graph: &mut ConversationGraph,
    parent_id: Uuid,
    title: &str,
    description: Option<&str>,
) -> ToolResultContent {
    // Validate parent exists and is a WorkItem.
    if !matches!(graph.node(parent_id), Some(Node::WorkItem { .. })) {
        return ToolResultContent::text(format!("Error: {parent_id} is not a valid work item"));
    }

    let task_id = Uuid::new_v4();
    let task = Node::WorkItem {
        id: task_id,
        kind: WorkItemKind::Task,
        title: title.to_string(),
        status: WorkItemStatus::Todo,
        description: description.map(String::from),
        created_at: Utc::now(),
    };
    graph.add_node(task);
    graph.emit(GraphEvent::WorkItemAdded {
        node_id: task_id,
        kind: WorkItemKind::Task,
    });
    let _ = graph.add_edge(task_id, parent_id, EdgeKind::SubtaskOf);
    ToolResultContent::text(format!(
        "Added task '{title}' under {parent_id} (id: {task_id})."
    ))
}

/// Update a `WorkItem`'s status and/or description, with upward propagation.
fn apply_update_work_item(
    graph: &mut ConversationGraph,
    id: Uuid,
    new_status: Option<&WorkItemStatus>,
    new_description: Option<&str>,
) -> ToolResultContent {
    if !matches!(graph.node(id), Some(Node::WorkItem { .. })) {
        return ToolResultContent::text(format!("Error: {id} is not a valid work item"));
    }

    if let Some(status) = new_status {
        if let Err(e) = graph.update_work_item_status(id, status.clone()) {
            return ToolResultContent::text(format!("Error updating status: {e}"));
        }
    }

    // Update description separately (no snapshot — not worth versioning for text edits).
    if let Some(desc) = new_description {
        if let Some(Node::WorkItem { description, .. }) = graph.node_mut(id) {
            *description = Some(desc.to_string());
        }
    }

    let status_str = new_status.map_or("unchanged".to_string(), |s| format!("{s:?}"));
    ToolResultContent::text(format!("Updated work item {id}: status → {status_str}."))
}

/// Create a `DependsOn` edge between two `Plan`-kind `WorkItem` nodes, with cycle detection.
fn apply_add_dependency(
    graph: &mut ConversationGraph,
    from_id: Uuid,
    to_id: Uuid,
) -> ToolResultContent {
    // Validate both are Plan-kind WorkItems.
    for (id, label) in [(from_id, "from_id"), (to_id, "to_id")] {
        if !matches!(
            graph.node(id),
            Some(Node::WorkItem {
                kind: WorkItemKind::Plan,
                ..
            })
        ) {
            return ToolResultContent::text(format!("Error: {label} ({id}) is not a valid plan"));
        }
    }

    // Cycle detection: DFS from `to_id` following `DependsOn` edges.
    if graph.has_dependency_path(to_id, from_id) {
        return ToolResultContent::text(format!(
            "Error: adding dependency {from_id} → {to_id} would create a cycle"
        ));
    }

    let _ = graph.add_edge(from_id, to_id, EdgeKind::DependsOn);
    graph.emit(GraphEvent::DependencyAdded { from_id, to_id });
    ToolResultContent::text(format!("Plan {from_id} now depends on {to_id}."))
}

#[cfg(test)]
#[path = "effects_tests.rs"]
mod tests;
