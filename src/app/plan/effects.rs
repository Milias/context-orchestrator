//! Side-effects for plan and task management tools.
//!
//! These run in `handle_tool_call_completed` after tool execution. They have
//! graph write access and produce enriched tool result content with UUIDs.

use crate::graph::tool_types::{ToolCallArguments, ToolResultContent};
use crate::graph::{ConversationGraph, EdgeKind, Node, WorkItemKind, WorkItemStatus};

use chrono::Utc;
use uuid::Uuid;

/// Result of applying a plan-related side-effect.
pub struct PlanEffectResult {
    /// Enriched tool result content (replaces the executor's placeholder).
    pub content: ToolResultContent,
    /// Optional status message for the TUI.
    pub status_message: Option<String>,
}

/// Apply plan-tool side-effects for a completed tool call.
/// Returns `Some` with enriched content if the tool is plan-related, `None` otherwise.
pub fn apply(graph: &mut ConversationGraph, tool_call_id: Uuid) -> Option<PlanEffectResult> {
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
) -> PlanEffectResult {
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
    let _ = graph.add_edge(wi_id, parent_message_id, EdgeKind::RelevantTo);
    PlanEffectResult {
        content: ToolResultContent::text(format!(
            "Created plan '{title}' (id: {wi_id}). Use add_task to decompose it into steps."
        )),
        status_message: Some(format!("Plan created: {title}")),
    }
}

/// Create a `Task`-kind `WorkItem` under a parent, link via `SubtaskOf`.
fn apply_add_task(
    graph: &mut ConversationGraph,
    parent_id: Uuid,
    title: &str,
    description: Option<&str>,
) -> PlanEffectResult {
    // Validate parent exists and is a WorkItem.
    match graph.node(parent_id) {
        Some(Node::WorkItem { .. }) => {}
        _ => {
            return PlanEffectResult {
                content: ToolResultContent::text(format!(
                    "Error: {parent_id} is not a valid work item"
                )),
                status_message: None,
            };
        }
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
    let _ = graph.add_edge(task_id, parent_id, EdgeKind::SubtaskOf);
    PlanEffectResult {
        content: ToolResultContent::text(format!(
            "Added task '{title}' under {parent_id} (id: {task_id})."
        )),
        status_message: Some(format!("Task added: {title}")),
    }
}

/// Update a `WorkItem`'s status and/or description, with upward propagation.
fn apply_update_work_item(
    graph: &mut ConversationGraph,
    id: Uuid,
    new_status: Option<&WorkItemStatus>,
    new_description: Option<&str>,
) -> PlanEffectResult {
    match graph.node(id) {
        Some(Node::WorkItem { .. }) => {}
        _ => {
            return PlanEffectResult {
                content: ToolResultContent::text(format!("Error: {id} is not a valid work item")),
                status_message: None,
            };
        }
    }

    if let Some(status) = new_status {
        if let Err(e) = graph.update_work_item_status(id, status.clone()) {
            return PlanEffectResult {
                content: ToolResultContent::text(format!("Error updating status: {e}")),
                status_message: None,
            };
        }
    }

    // Update description separately (no snapshot — not worth versioning for text edits).
    if let Some(desc) = new_description {
        if let Some(Node::WorkItem { description, .. }) = graph.node_mut(id) {
            *description = Some(desc.to_string());
        }
    }

    let status_str = new_status.map_or("unchanged".to_string(), |s| format!("{s:?}"));
    PlanEffectResult {
        content: ToolResultContent::text(format!("Updated work item {id}: status → {status_str}.")),
        status_message: Some(format!("Work item updated: {id}")),
    }
}

/// Create a `DependsOn` edge between two `Plan`-kind `WorkItem` nodes, with cycle detection.
fn apply_add_dependency(
    graph: &mut ConversationGraph,
    from_id: Uuid,
    to_id: Uuid,
) -> PlanEffectResult {
    // Validate both are Plan-kind WorkItems.
    for (id, label) in [(from_id, "from_id"), (to_id, "to_id")] {
        match graph.node(id) {
            Some(Node::WorkItem {
                kind: WorkItemKind::Plan,
                ..
            }) => {}
            _ => {
                return PlanEffectResult {
                    content: ToolResultContent::text(format!(
                        "Error: {label} ({id}) is not a valid plan"
                    )),
                    status_message: None,
                };
            }
        }
    }

    // Cycle detection: DFS from to_id following DependsOn edges.
    // If we reach from_id, adding this edge would create a cycle.
    if graph.has_dependency_path(to_id, from_id) {
        return PlanEffectResult {
            content: ToolResultContent::text(format!(
                "Error: adding dependency {from_id} → {to_id} would create a cycle"
            )),
            status_message: None,
        };
    }

    let _ = graph.add_edge(from_id, to_id, EdgeKind::DependsOn);
    PlanEffectResult {
        content: ToolResultContent::text(format!("Plan {from_id} now depends on {to_id}.")),
        status_message: Some("Dependency added".to_string()),
    }
}

#[cfg(test)]
#[path = "effects_tests.rs"]
mod tests;
