use super::*;

use crate::graph::{ConversationGraph, EdgeKind, Node, Role, WorkItemKind, WorkItemStatus};
use chrono::Utc;
use uuid::Uuid;

/// Helper: create a minimal graph with a user message on the main branch.
fn graph_with_message() -> (ConversationGraph, Uuid) {
    let mut graph = ConversationGraph::new("sys");
    let root = graph.branch_leaf("main").unwrap();
    let msg = Node::Message {
        id: Uuid::new_v4(),
        role: Role::User,
        content: "test".to_string(),
        created_at: Utc::now(),
        model: None,
        input_tokens: None,
        output_tokens: None,
        stop_reason: None,
    };
    let msg_id = graph.add_message(root, msg).unwrap();
    (graph, msg_id)
}

/// Helper: add a plan to the graph with a `RelevantTo` edge to the parent message.
fn add_plan(graph: &mut ConversationGraph, title: &str, parent_msg: Uuid) -> Uuid {
    let id = Uuid::new_v4();
    graph.add_node(Node::WorkItem {
        id,
        kind: WorkItemKind::Plan,
        title: title.to_string(),
        status: WorkItemStatus::Active,
        description: None,
        completion_confidence: None,
        created_at: Utc::now(),
    });
    let _ = graph.add_edge(id, parent_msg, EdgeKind::RelevantTo);
    id
}

/// Helper: add a task under a plan.
fn add_task(graph: &mut ConversationGraph, title: &str, plan_id: Uuid) -> Uuid {
    let id = Uuid::new_v4();
    graph.add_node(Node::WorkItem {
        id,
        kind: WorkItemKind::Task,
        title: title.to_string(),
        status: WorkItemStatus::Todo,
        description: Some("Do the thing".to_string()),
        completion_confidence: None,
        created_at: Utc::now(),
    });
    let _ = graph.add_edge(id, plan_id, EdgeKind::SubtaskOf);
    id
}

/// Bug: `build_context` includes unrelated plans in the `## Plan:` section,
/// polluting the task agent's scoped context with the wrong plan hierarchy.
/// When two plans exist but the task belongs to only one, the `## Plan:` header
/// section must only show the parent plan. If both appear in the plan section,
/// the agent sees sibling tasks from the wrong plan and may act on them.
/// (Unrelated plans may still appear as low-priority supplementary summaries
/// from the scoring pipeline, which is acceptable — the bug is in the plan section.)
#[test]
fn build_context_plan_section_scoped_to_parent() {
    let (mut graph, msg_id) = graph_with_message();

    let plan_a = add_plan(&mut graph, "Plan Alpha", msg_id);
    let _plan_b = add_plan(&mut graph, "Plan Beta", msg_id);

    let task = add_task(&mut graph, "Alpha Task 1", plan_a);

    let agent_id = Uuid::new_v4();
    let result = build_context(&graph, task, agent_id);

    let system = result.system_prompt.expect("should have system prompt");

    // The `## Plan:` section should only reference Plan Alpha.
    assert!(
        system.contains("## Plan: \"Plan Alpha\""),
        "plan section should reference the parent plan 'Plan Alpha'"
    );
    assert!(
        !system.contains("## Plan: \"Plan Beta\""),
        "plan section must NOT reference unrelated 'Plan Beta' — \
         scoping should only show the parent plan hierarchy"
    );
}

/// Bug: `build_context` panics when given a work item with no parent plan.
/// An orphan task (no `SubtaskOf` edge) should produce a valid context without
/// the plan section, not crash. This can happen when tasks are created directly
/// without a plan wrapper.
#[test]
fn build_context_handles_orphan_task_without_panic() {
    let (mut graph, _msg_id) = graph_with_message();

    let orphan_id = Uuid::new_v4();
    graph.add_node(Node::WorkItem {
        id: orphan_id,
        kind: WorkItemKind::Task,
        title: "Orphan Task".to_string(),
        status: WorkItemStatus::Active,
        description: Some("No parent plan".to_string()),
        completion_confidence: None,
        created_at: Utc::now(),
    });

    let agent_id = Uuid::new_v4();
    // This must not panic.
    let result = build_context(&graph, orphan_id, agent_id);

    let system = result.system_prompt.expect("should have system prompt");
    assert!(
        system.contains("Orphan Task"),
        "system prompt should still mention the task title"
    );
    // No plan section should be present since there's no parent plan.
    assert!(
        !system.contains("## Plan:"),
        "system prompt must not contain a Plan section for an orphan task"
    );
}
