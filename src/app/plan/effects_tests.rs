use crate::graph::tool_types::{ToolCallArguments, ToolCallStatus};
use crate::graph::{ConversationGraph, EdgeKind, Node, Role, WorkItemKind, WorkItemStatus};

use chrono::Utc;
use uuid::Uuid;

/// Helper: create a graph with a user message and return `(graph, msg_id)`.
/// Many `plan_effects` tests need a parent message to anchor tool calls.
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

/// Helper: create a `Plan` `WorkItem` in the graph and return its ID.
fn add_plan(graph: &mut ConversationGraph, title: &str) -> Uuid {
    let id = Uuid::new_v4();
    graph.add_node(Node::WorkItem {
        id,
        kind: WorkItemKind::Plan,
        title: title.to_string(),
        status: WorkItemStatus::Todo,
        description: None,
        completion_confidence: None,
        created_at: Utc::now(),
    });
    id
}

/// Helper: add a tool call to the graph and run `plan_effects::apply`.
/// Returns the enriched content text from the effect result.
fn apply_tool_call(
    graph: &mut ConversationGraph,
    msg_id: Uuid,
    arguments: ToolCallArguments,
) -> String {
    let tc_id = graph.add_tool_call(Uuid::new_v4(), msg_id, arguments, None);
    graph
        .update_tool_call_status(tc_id, ToolCallStatus::Completed, Some(Utc::now()))
        .unwrap();
    let result = super::apply(graph, tc_id);
    result
        .expect("plan effect should return Some for plan tools")
        .text_content()
        .to_string()
}

/// Bug: missing `SubtaskOf` edge or wrong direction after `AddTask`.
/// The child `WorkItem` must exist AND have a `SubtaskOf` edge from child to parent.
#[test]
fn test_add_task_creates_work_item_with_subtask_edge() {
    let (mut graph, msg_id) = graph_with_message();
    let plan_id = add_plan(&mut graph, "My Plan");

    let content = apply_tool_call(
        &mut graph,
        msg_id,
        ToolCallArguments::AddTask {
            parent_id: plan_id,
            title: "Step 1".to_string(),
            description: Some("First step".to_string()),
        },
    );

    // The response should confirm task creation.
    assert!(
        content.contains("Added task"),
        "response should confirm creation, got: {content}"
    );

    // A child WorkItem should exist.
    let children = graph.children_of(plan_id);
    assert_eq!(children.len(), 1, "plan should have exactly 1 child task");

    let child_id = children[0];
    if let Some(Node::WorkItem {
        title,
        kind,
        status,
        description,
        ..
    }) = graph.node(child_id)
    {
        assert_eq!(title, "Step 1");
        assert_eq!(*kind, WorkItemKind::Task);
        assert_eq!(*status, WorkItemStatus::Todo);
        assert_eq!(description.as_deref(), Some("First step"));
    } else {
        panic!("child node should be a WorkItem");
    }

    // SubtaskOf edge must go child --> parent.
    assert_eq!(
        graph.parent_of(child_id),
        Some(plan_id),
        "SubtaskOf edge should point from child to parent"
    );
}

/// Bug: missing parent existence check in `AddTask`.
/// Calling `AddTask` with a non-existent `parent_id` must return an error instead of
/// silently creating an orphan task.
#[test]
fn test_add_task_with_nonexistent_parent_returns_error() {
    let (mut graph, msg_id) = graph_with_message();
    let fake_parent = Uuid::new_v4();

    let content = apply_tool_call(
        &mut graph,
        msg_id,
        ToolCallArguments::AddTask {
            parent_id: fake_parent,
            title: "Orphan task".to_string(),
            description: None,
        },
    );

    assert!(
        content.contains("Error"),
        "should return an error for non-existent parent, got: {content}"
    );

    // No Task WorkItems should have been created.
    let tasks = graph.nodes_by(|n| {
        matches!(
            n,
            Node::WorkItem {
                kind: WorkItemKind::Task,
                ..
            }
        )
    });
    assert!(tasks.is_empty(), "no orphan task should be created");
}

/// Bug: missing `DependsOn` edge creation in `AddDependency`.
/// After calling `AddDependency`, a `DependsOn` edge must exist from `from_id` to `to_id`.
#[test]
fn test_add_dependency_creates_depends_on_edge() {
    let (mut graph, msg_id) = graph_with_message();
    let plan_a = add_plan(&mut graph, "Plan A");
    let plan_b = add_plan(&mut graph, "Plan B");

    let content = apply_tool_call(
        &mut graph,
        msg_id,
        ToolCallArguments::AddDependency {
            from_id: plan_a,
            to_id: plan_b,
        },
    );

    assert!(
        content.contains("depends on"),
        "response should confirm dependency, got: {content}"
    );

    let deps = graph.dependencies_of(plan_a);
    assert_eq!(deps, vec![plan_b], "Plan A should depend on Plan B");

    // Reverse direction should be empty.
    let reverse_deps = graph.dependencies_of(plan_b);
    assert!(
        reverse_deps.is_empty(),
        "Plan B should not depend on Plan A"
    );
}

/// Bug: missing cycle detection in `AddDependency`.
/// If A depends on B, adding B depends on A must fail with a cycle error
/// instead of creating a circular dependency.
#[test]
fn test_add_dependency_with_cycle_returns_error() {
    let (mut graph, msg_id) = graph_with_message();
    let plan_a = add_plan(&mut graph, "Plan A");
    let plan_b = add_plan(&mut graph, "Plan B");

    // First dependency: A depends on B (should succeed).
    graph.add_edge(plan_a, plan_b, EdgeKind::DependsOn).unwrap();

    // Attempt the reverse: B depends on A (should detect cycle).
    let content = apply_tool_call(
        &mut graph,
        msg_id,
        ToolCallArguments::AddDependency {
            from_id: plan_b,
            to_id: plan_a,
        },
    );

    assert!(
        content.contains("cycle"),
        "should reject cyclic dependency, got: {content}"
    );

    // Plan B should NOT have gained a dependency on Plan A.
    let b_deps = graph.dependencies_of(plan_b);
    assert!(b_deps.is_empty(), "cyclic edge should not have been added");
}

/// Bug: `Plan` tool creates a `WorkItem` but the `RelevantTo` edge
/// linking it to the parent message is missing — plan becomes an
/// orphan invisible to context building.
#[test]
fn test_plan_creates_work_item_with_relevant_to_edge() {
    let (mut graph, msg_id) = graph_with_message();

    let content = apply_tool_call(
        &mut graph,
        msg_id,
        ToolCallArguments::Plan {
            title: "Auth Refactor".to_string(),
            description: Some("Rewrite auth module".to_string()),
        },
    );

    assert!(
        content.contains("Auth Refactor"),
        "should mention plan title, got: {content}"
    );

    // A Plan WorkItem should exist.
    let plans = graph.nodes_by(|n| {
        matches!(
            n,
            Node::WorkItem {
                kind: WorkItemKind::Plan,
                ..
            }
        )
    });
    assert_eq!(plans.len(), 1, "exactly one Plan should exist");
    let plan_id = plans[0].id();

    // RelevantTo edge must link plan → parent message.
    let has_edge = graph
        .edges
        .iter()
        .any(|e| e.from == plan_id && e.to == msg_id && e.kind == EdgeKind::RelevantTo);
    assert!(
        has_edge,
        "Plan should have RelevantTo edge to parent message"
    );
}

/// Bug: `UpdateWorkItem` with a status change silently no-ops — the
/// work item stays in its original status.
#[test]
fn test_update_work_item_changes_status() {
    let (mut graph, msg_id) = graph_with_message();
    let plan_id = add_plan(&mut graph, "My Plan");

    let content = apply_tool_call(
        &mut graph,
        msg_id,
        ToolCallArguments::UpdateWorkItem {
            id: plan_id,
            status: Some(WorkItemStatus::Active),
            description: None,
            confidence: None,
        },
    );

    assert!(
        !content.contains("Error"),
        "should not error, got: {content}"
    );

    if let Some(Node::WorkItem { status, .. }) = graph.node(plan_id) {
        assert_eq!(*status, WorkItemStatus::Active, "status should be Active");
    } else {
        panic!("plan node should exist");
    }
}

/// Bug: `UpdateWorkItem` with a description change silently no-ops.
#[test]
fn test_update_work_item_changes_description() {
    let (mut graph, msg_id) = graph_with_message();
    let plan_id = add_plan(&mut graph, "My Plan");

    let content = apply_tool_call(
        &mut graph,
        msg_id,
        ToolCallArguments::UpdateWorkItem {
            id: plan_id,
            status: None,
            description: Some("Updated description".to_string()),
            confidence: None,
        },
    );

    assert!(
        !content.contains("Error"),
        "should not error, got: {content}"
    );

    if let Some(Node::WorkItem { description, .. }) = graph.node(plan_id) {
        assert_eq!(
            description.as_deref(),
            Some("Updated description"),
            "description should be updated"
        );
    } else {
        panic!("plan node should exist");
    }
}

/// Bug: `UpdateWorkItem` with a non-existent ID silently succeeds
/// instead of returning an error, misleading the agent.
#[test]
fn test_update_work_item_nonexistent_returns_error() {
    let (mut graph, msg_id) = graph_with_message();
    let fake_id = Uuid::new_v4();

    let content = apply_tool_call(
        &mut graph,
        msg_id,
        ToolCallArguments::UpdateWorkItem {
            id: fake_id,
            status: Some(WorkItemStatus::Done),
            description: None,
            confidence: None,
        },
    );

    assert!(
        content.contains("Error") || content.contains("not found"),
        "should return error for non-existent ID, got: {content}"
    );
}

/// Helper: add a task under a plan and return (plan_id, task_id).
fn add_plan_with_task(graph: &mut ConversationGraph) -> (Uuid, Uuid) {
    let plan_id = add_plan(graph, "Test Plan");
    let task_id = Uuid::new_v4();
    graph.add_node(Node::WorkItem {
        id: task_id,
        kind: WorkItemKind::Task,
        title: "Test Task".to_string(),
        status: WorkItemStatus::Active,
        description: None,
        completion_confidence: None,
        created_at: Utc::now(),
    });
    let _ = graph.add_edge(task_id, plan_id, EdgeKind::SubtaskOf);
    (plan_id, task_id)
}

/// Bug: `update_work_item` with confidence="high" does not auto-accept completion,
/// leaving the task in Active state and emitting a review event when it should
/// transition to Done immediately. High-confidence completions from agents that
/// have verified their work should bypass review.
#[test]
fn test_high_confidence_auto_accepts_done() {
    let (mut graph, msg_id) = graph_with_message();
    let (_plan_id, task_id) = add_plan_with_task(&mut graph);

    let content = apply_tool_call(
        &mut graph,
        msg_id,
        ToolCallArguments::UpdateWorkItem {
            id: task_id,
            status: Some(WorkItemStatus::Done),
            description: None,
            confidence: Some("high".to_string()),
        },
    );

    assert!(
        !content.contains("Proposed") && !content.contains("Awaiting review"),
        "high confidence should auto-accept, not propose review, got: {content}"
    );

    if let Some(Node::WorkItem { status, .. }) = graph.node(task_id) {
        assert_eq!(
            *status,
            WorkItemStatus::Done,
            "high-confidence Done should transition immediately to Done"
        );
    } else {
        panic!("task node should exist");
    }
}

/// Bug: `update_work_item` with confidence="moderate" transitions to Done instead
/// of staying Active and routing for review. Moderate confidence means the agent
/// is uncertain — the work should stay Active with a CompletionProposed event
/// emitted so a reviewer can verify. If it auto-transitions, unfinished work
/// gets marked complete.
#[test]
fn test_moderate_confidence_stays_active_and_proposes() {
    let (mut graph, msg_id) = graph_with_message();
    let (_plan_id, task_id) = add_plan_with_task(&mut graph);

    let content = apply_tool_call(
        &mut graph,
        msg_id,
        ToolCallArguments::UpdateWorkItem {
            id: task_id,
            status: Some(WorkItemStatus::Done),
            description: None,
            confidence: Some("moderate".to_string()),
        },
    );

    assert!(
        content.contains("Proposed") || content.contains("Awaiting review"),
        "moderate confidence should propose completion for review, got: {content}"
    );

    if let Some(Node::WorkItem { status, .. }) = graph.node(task_id) {
        assert_eq!(
            *status,
            WorkItemStatus::Active,
            "moderate-confidence Done should NOT transition to Done — must stay Active for review"
        );
    } else {
        panic!("task node should exist");
    }
}
