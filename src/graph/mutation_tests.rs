use super::*;
use crate::graph::tool_types::{ToolCallArguments, ToolCallStatus};

/// Bug: `update_tool_call_status` bypasses `mutate_node`, so no history is recorded.
#[test]
fn test_tool_call_status_change_captures_snapshot() {
    let mut graph = ConversationGraph::new("sys");
    let root = graph.branch_leaf("main").unwrap();
    let msg = Node::Message {
        id: Uuid::new_v4(),
        role: Role::User,
        content: "hi".to_string(),
        created_at: Utc::now(),
        model: None,
        input_tokens: None,
        output_tokens: None,
        stop_reason: None,
    };
    let msg_id = graph.add_message(root, msg).unwrap();

    let tc_id = graph.add_tool_call(
        Uuid::new_v4(),
        msg_id,
        ToolCallArguments::Unknown {
            tool_name: "test".to_string(),
            raw_json: "{}".to_string(),
        },
        None,
    );

    // add_tool_call creates Pending then transitions to Running → 1 snapshot
    assert_eq!(graph.node_history(tc_id).len(), 1);
    let first_snapshot = &graph.node_history(tc_id)[0];
    if let Node::ToolCall { status, .. } = &first_snapshot.node {
        assert_eq!(
            *status,
            ToolCallStatus::Pending,
            "snapshot should capture Pending state"
        );
    } else {
        panic!("snapshot should be a ToolCall");
    }

    // Now transition Running → Completed
    graph
        .update_tool_call_status(tc_id, ToolCallStatus::Completed, Some(Utc::now()))
        .unwrap();
    assert_eq!(graph.node_history(tc_id).len(), 2);
    if let Node::ToolCall { status, .. } = &graph.node_history(tc_id)[1].node {
        assert_eq!(
            *status,
            ToolCallStatus::Running,
            "second snapshot should capture Running state"
        );
    } else {
        panic!("snapshot should be a ToolCall");
    }
}

/// Bug: snapshot taken AFTER mutation, so the old description is lost.
#[test]
fn test_background_task_snapshot_preserves_old_description() {
    let mut graph = ConversationGraph::new("sys");
    let id = Uuid::new_v4();
    graph.add_node(Node::BackgroundTask {
        id,
        kind: BackgroundTaskKind::AgentPhase,
        status: TaskStatus::Running,
        description: "Building context...".to_string(),
        created_at: Utc::now(),
        updated_at: Utc::now(),
    });

    graph
        .update_background_task_status(id, TaskStatus::Completed, "Done".to_string())
        .unwrap();

    let history = graph.node_history(id);
    assert_eq!(history.len(), 1);
    if let Node::BackgroundTask {
        status,
        description,
        ..
    } = &history[0].node
    {
        assert_eq!(*status, TaskStatus::Running);
        assert_eq!(description, "Building context...");
    } else {
        panic!("snapshot should be a BackgroundTask");
    }
}

/// Bug: bulk `transition_running_tasks` bypasses `mutate_node`, no snapshots.
#[test]
fn test_transition_running_tasks_creates_per_node_snapshots() {
    let mut graph = ConversationGraph::new("sys");
    let id1 = Uuid::new_v4();
    let id2 = Uuid::new_v4();
    for (id, desc) in [(id1, "task 1"), (id2, "task 2")] {
        graph.add_node(Node::BackgroundTask {
            id,
            kind: BackgroundTaskKind::AgentPhase,
            status: TaskStatus::Running,
            description: desc.to_string(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        });
    }

    graph.stop_running_tasks();

    assert_eq!(
        graph.node_history(id1).len(),
        1,
        "task 1 should have 1 snapshot"
    );
    assert_eq!(
        graph.node_history(id2).len(),
        1,
        "task 2 should have 1 snapshot"
    );
    if let Node::BackgroundTask { status, .. } = &graph.node_history(id1)[0].node {
        assert_eq!(
            *status,
            TaskStatus::Running,
            "snapshot should capture Running before Stopped"
        );
    } else {
        panic!("snapshot should be a BackgroundTask");
    }
}

/// Bug: removing a node leaves orphaned history entries.
#[test]
fn test_remove_nodes_cleans_history() {
    let mut graph = ConversationGraph::new("sys");
    let id = Uuid::new_v4();
    graph.add_node(Node::BackgroundTask {
        id,
        kind: BackgroundTaskKind::AgentPhase,
        status: TaskStatus::Running,
        description: "task".to_string(),
        created_at: Utc::now(),
        updated_at: Utc::now(),
    });
    graph
        .update_background_task_status(id, TaskStatus::Completed, "done".to_string())
        .unwrap();
    assert_eq!(graph.node_history(id).len(), 1);

    graph.remove_nodes_by(|n| n.id() == id);
    assert!(
        graph.node_history(id).is_empty(),
        "history should be cleaned on removal"
    );
}

/// Bug: parent not auto-completing when all children are Done.
/// `propagate_status` must walk `SubtaskOf` edges upward and set parent to Done
/// when every sibling is Done.
#[test]
fn test_propagate_status_completes_parent_when_all_children_done() {
    let mut graph = ConversationGraph::new("sys");

    // Create a Plan parent.
    let parent_id = Uuid::new_v4();
    graph.add_node(Node::WorkItem {
        id: parent_id,
        kind: WorkItemKind::Plan,
        title: "Parent plan".to_string(),
        status: WorkItemStatus::Todo,
        description: None,
        created_at: Utc::now(),
    });

    // Create 2 Task children linked via SubtaskOf.
    let child1_id = Uuid::new_v4();
    let child2_id = Uuid::new_v4();
    for (id, title) in [(child1_id, "Task 1"), (child2_id, "Task 2")] {
        graph.add_node(Node::WorkItem {
            id,
            kind: WorkItemKind::Task,
            title: title.to_string(),
            status: WorkItemStatus::Todo,
            description: None,
            created_at: Utc::now(),
        });
        graph.add_edge(id, parent_id, EdgeKind::SubtaskOf).unwrap();
    }

    // Mark both children as Done.
    graph
        .update_work_item_status(child1_id, WorkItemStatus::Done)
        .unwrap();
    graph
        .update_work_item_status(child2_id, WorkItemStatus::Done)
        .unwrap();

    // Parent should have auto-transitioned to Done.
    if let Some(Node::WorkItem { status, .. }) = graph.node(parent_id) {
        assert_eq!(
            *status,
            WorkItemStatus::Done,
            "parent should auto-complete when all children are Done"
        );
    } else {
        panic!("parent node should exist and be a WorkItem");
    }
}

/// Bug: wrong `SubtaskOf` edge direction in `children_of` query.
/// `children_of(parent)` must return children that have `child --SubtaskOf--> parent`
/// edges, and `parent_of(child)` must return the parent.
#[test]
fn test_children_of_returns_subtask_children() {
    let mut graph = ConversationGraph::new("sys");

    let parent_id = Uuid::new_v4();
    graph.add_node(Node::WorkItem {
        id: parent_id,
        kind: WorkItemKind::Plan,
        title: "Parent".to_string(),
        status: WorkItemStatus::Todo,
        description: None,
        created_at: Utc::now(),
    });

    let child_id = Uuid::new_v4();
    graph.add_node(Node::WorkItem {
        id: child_id,
        kind: WorkItemKind::Task,
        title: "Child".to_string(),
        status: WorkItemStatus::Todo,
        description: None,
        created_at: Utc::now(),
    });

    // Edge direction: child --SubtaskOf--> parent.
    graph
        .add_edge(child_id, parent_id, EdgeKind::SubtaskOf)
        .unwrap();

    let children = graph.children_of(parent_id);
    assert_eq!(
        children,
        vec![child_id],
        "children_of should return the child"
    );

    let parent = graph.parent_of(child_id);
    assert_eq!(
        parent,
        Some(parent_id),
        "parent_of should return the parent"
    );
}

/// Bug: `add_tool_call` Pending→Running must produce a Pending snapshot.
#[test]
fn test_add_tool_call_captures_pending_snapshot() {
    let mut graph = ConversationGraph::new("sys");
    let root = graph.branch_leaf("main").unwrap();
    let msg = Node::Message {
        id: Uuid::new_v4(),
        role: Role::User,
        content: "hi".to_string(),
        created_at: Utc::now(),
        model: None,
        input_tokens: None,
        output_tokens: None,
        stop_reason: None,
    };
    let msg_id = graph.add_message(root, msg).unwrap();

    let tc_id = graph.add_tool_call(
        Uuid::new_v4(),
        msg_id,
        ToolCallArguments::Unknown {
            tool_name: "test".to_string(),
            raw_json: "{}".to_string(),
        },
        None,
    );

    // Current state should be Running
    if let Some(Node::ToolCall { status, .. }) = graph.node(tc_id) {
        assert_eq!(*status, ToolCallStatus::Running);
    }
    // History should have exactly 1 snapshot of the Pending state
    let history = graph.node_history(tc_id);
    assert_eq!(history.len(), 1);
    if let Node::ToolCall { status, .. } = &history[0].node {
        assert_eq!(*status, ToolCallStatus::Pending);
    } else {
        panic!("snapshot should be a ToolCall");
    }
}
