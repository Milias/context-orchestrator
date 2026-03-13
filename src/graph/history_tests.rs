use super::*;

/// Bug: `history` field missing from `ConversationGraphRaw`, silently dropped on save.
#[test]
fn test_history_survives_serde_roundtrip() {
    let mut graph = ConversationGraph::new("sys");
    let id = Uuid::new_v4();
    graph.add_node(Node::BackgroundTask {
        id,
        kind: BackgroundTaskKind::AgentPhase,
        status: TaskStatus::Running,
        description: "phase".to_string(),
        created_at: Utc::now(),
        updated_at: Utc::now(),
    });
    graph
        .update_background_task_status(id, TaskStatus::Completed, "done".to_string())
        .unwrap();
    assert_eq!(graph.node_history(id).len(), 1);

    // Serialize and deserialize
    let json = serde_json::to_string(&graph).unwrap();
    let restored: ConversationGraph = serde_json::from_str(&json).unwrap();

    assert_eq!(
        restored.node_history(id).len(),
        1,
        "history must survive serde roundtrip"
    );
    if let Node::BackgroundTask { status, .. } = &restored.node_history(id)[0].node {
        assert_eq!(
            *status,
            TaskStatus::Running,
            "snapshot should preserve Running state"
        );
    } else {
        panic!("snapshot should be a BackgroundTask");
    }
}

/// Bug: `node_history` returns snapshots in wrong order (newest-first instead of oldest-first).
#[test]
fn test_node_history_returns_chronological_order() {
    let mut graph = ConversationGraph::new("sys");
    let id = Uuid::new_v4();
    graph.add_node(Node::BackgroundTask {
        id,
        kind: BackgroundTaskKind::AgentPhase,
        status: TaskStatus::Pending,
        description: "pending".to_string(),
        created_at: Utc::now(),
        updated_at: Utc::now(),
    });

    // Pending → Running
    graph
        .update_background_task_status(id, TaskStatus::Running, "running".to_string())
        .unwrap();
    // Running → Completed
    graph
        .update_background_task_status(id, TaskStatus::Completed, "done".to_string())
        .unwrap();

    let history = graph.node_history(id);
    assert_eq!(history.len(), 2);

    // First snapshot should be the oldest (Pending)
    if let Node::BackgroundTask { status, .. } = &history[0].node {
        assert_eq!(
            *status,
            TaskStatus::Pending,
            "first snapshot should be oldest (Pending)"
        );
    }
    // Second snapshot should be newer (Running)
    if let Node::BackgroundTask { status, .. } = &history[1].node {
        assert_eq!(
            *status,
            TaskStatus::Running,
            "second snapshot should be Running"
        );
    }

    // Timestamps must be non-decreasing
    assert!(history[0].captured_at <= history[1].captured_at);
}

/// Bug: `node_history` panics on nodes with no history instead of returning empty slice.
#[test]
fn test_node_history_empty_for_unversioned() {
    let graph = ConversationGraph::new("sys");
    let nonexistent_id = Uuid::new_v4();
    assert!(
        graph.node_history(nonexistent_id).is_empty(),
        "node_history should return empty slice for unknown nodes"
    );
}
