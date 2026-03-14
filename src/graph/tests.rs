use super::*;

#[test]
fn test_new_creates_root_and_main_branch() {
    let graph = ConversationGraph::new("You are helpful.");
    assert_eq!(graph.active_branch(), "main");

    let history = graph.get_branch_history("main").unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].content(), "You are helpful.");
}

#[test]
fn test_add_message_and_history() {
    let mut graph = ConversationGraph::new("System prompt");
    let root_id = graph.branch_leaf("main").unwrap();

    let user_msg = Node::Message {
        id: Uuid::new_v4(),
        role: Role::User,
        content: "Hello".to_string(),
        created_at: Utc::now(),
        model: None,
        input_tokens: None,
        output_tokens: None,
        stop_reason: None,
    };
    let user_id = graph.add_message(root_id, user_msg).unwrap();

    let asst_msg = Node::Message {
        id: Uuid::new_v4(),
        role: Role::Assistant,
        content: "Hi there".to_string(),
        created_at: Utc::now(),
        model: Some("claude".to_string()),
        input_tokens: Some(25),
        output_tokens: Some(10),
        stop_reason: None,
    };
    let _asst_id = graph.add_message(user_id, asst_msg).unwrap();

    let history = graph.get_branch_history("main").unwrap();
    assert_eq!(history.len(), 3);
    assert_eq!(history[0].content(), "System prompt");
    assert_eq!(history[1].content(), "Hello");
    assert_eq!(history[2].content(), "Hi there");
}

#[test]
fn test_serde_roundtrip() {
    let mut graph = ConversationGraph::new("System prompt");
    let root_id = graph.branch_leaf("main").unwrap();

    let msg = Node::Message {
        id: Uuid::new_v4(),
        role: Role::User,
        content: "Hello".to_string(),
        created_at: Utc::now(),
        model: None,
        input_tokens: None,
        output_tokens: None,
        stop_reason: None,
    };
    graph.add_message(root_id, msg).unwrap();

    let json = serde_json::to_string_pretty(&graph).unwrap();
    let restored: ConversationGraph = serde_json::from_str(&json).unwrap();

    assert_eq!(restored.active_branch(), graph.active_branch());

    let orig_history = graph.get_branch_history("main").unwrap();
    let rest_history = restored.get_branch_history("main").unwrap();
    assert_eq!(orig_history.len(), rest_history.len());
    for (a, b) in orig_history.iter().zip(rest_history.iter()) {
        assert_eq!(a.id(), b.id());
        assert_eq!(a.content(), b.content());
    }
}

#[test]
fn test_add_node_without_edges() {
    let mut graph = ConversationGraph::new("System prompt");
    let work_item = Node::WorkItem {
        id: Uuid::new_v4(),
        title: "Fix the bug".to_string(),
        kind: WorkItemKind::Task,
        status: WorkItemStatus::Todo,
        description: None,
        completion_confidence: None,
        created_at: Utc::now(),
    };
    let wi_id = graph.add_node(work_item);
    let found = graph.nodes_by(|n| matches!(n, Node::WorkItem { .. }));
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].id(), wi_id);
}

#[test]
fn test_typed_edges() {
    let mut graph = ConversationGraph::new("System prompt");
    let root_id = graph.branch_leaf("main").unwrap();

    let wi = Node::WorkItem {
        id: Uuid::new_v4(),
        title: "Task".to_string(),
        kind: WorkItemKind::Task,
        status: WorkItemStatus::Active,
        description: None,
        completion_confidence: None,
        created_at: Utc::now(),
    };
    let wi_id = graph.add_node(wi);

    graph
        .add_edge(wi_id, root_id, EdgeKind::RelevantTo)
        .unwrap();
    assert_eq!(graph.edges.len(), 1);
    assert_eq!(graph.edges[0].kind, EdgeKind::RelevantTo);
}

#[test]
fn test_update_background_task_status() {
    let mut graph = ConversationGraph::new("System prompt");
    let id = Uuid::new_v4();

    let task = Node::BackgroundTask {
        id,
        kind: BackgroundTaskKind::GitIndex,
        status: TaskStatus::Running,
        description: "Indexing...".to_string(),
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };
    graph.add_node(task);
    assert_eq!(
        graph
            .nodes_by(|n| matches!(
                n,
                Node::BackgroundTask {
                    status: TaskStatus::Running,
                    ..
                }
            ))
            .len(),
        1
    );

    graph
        .update_background_task_status(id, TaskStatus::Completed, "Done".to_string())
        .unwrap();
    assert_eq!(
        graph
            .nodes_by(|n| matches!(
                n,
                Node::BackgroundTask {
                    status: TaskStatus::Completed,
                    ..
                }
            ))
            .len(),
        1
    );
    assert_eq!(
        graph
            .nodes_by(|n| matches!(
                n,
                Node::BackgroundTask {
                    status: TaskStatus::Running,
                    ..
                }
            ))
            .len(),
        0
    );
}

#[test]
fn test_remove_nodes_by() {
    let mut graph = ConversationGraph::new("System prompt");
    let root_id = graph.branch_leaf("main").unwrap();

    let gf1 = Node::GitFile {
        id: Uuid::new_v4(),
        path: "src/main.rs".to_string(),
        status: GitFileStatus::Tracked,
        updated_at: Utc::now(),
    };
    let gf1_id = graph.add_node(gf1);
    graph.add_edge(gf1_id, root_id, EdgeKind::Indexes).unwrap();

    let gf2 = Node::GitFile {
        id: Uuid::new_v4(),
        path: "src/lib.rs".to_string(),
        status: GitFileStatus::Modified,
        updated_at: Utc::now(),
    };
    graph.add_node(gf2);

    assert_eq!(
        graph.nodes_by(|n| matches!(n, Node::GitFile { .. })).len(),
        2
    );

    graph.remove_nodes_by(|n| matches!(n, Node::GitFile { .. }));

    assert_eq!(
        graph.nodes_by(|n| matches!(n, Node::GitFile { .. })).len(),
        0
    );
    assert!(graph.edges.is_empty());
}

#[test]
fn test_think_block_not_in_history() {
    let mut graph = ConversationGraph::new("System prompt");
    let root_id = graph.branch_leaf("main").unwrap();

    let asst_id = Uuid::new_v4();
    let asst = Node::Message {
        id: asst_id,
        role: Role::Assistant,
        content: "Hello".to_string(),
        created_at: Utc::now(),
        model: None,
        input_tokens: None,
        output_tokens: None,
        stop_reason: None,
    };
    graph.add_message(root_id, asst).unwrap();

    let think = Node::ThinkBlock {
        id: Uuid::new_v4(),
        content: "reasoning...".to_string(),
        parent_message_id: asst_id,
        created_at: Utc::now(),
    };
    let think_id = graph.add_node(think);
    graph
        .add_edge(think_id, asst_id, EdgeKind::ThinkingOf)
        .unwrap();

    // ThinkBlock should NOT appear in branch history (it has no RespondsTo edge)
    let history = graph.get_branch_history("main").unwrap();
    assert_eq!(history.len(), 2); // system + assistant only
    assert!(!history.iter().any(|n| matches!(n, Node::ThinkBlock { .. })));

    // But has_think_block should detect it
    assert!(graph.has_think_block(asst_id));
    assert!(!graph.has_think_block(root_id));
}

/// Bug: `add_reply` updates the branch pointer, corrupting the conversational
/// branch when task agents record messages.
#[test]
fn test_add_reply_does_not_update_branch() {
    let mut graph = ConversationGraph::new("System prompt");
    let root_id = graph.branch_leaf("main").unwrap();

    // add_message updates the branch leaf.
    let user_msg = Node::Message {
        id: Uuid::new_v4(),
        role: Role::User,
        content: "Hello".to_string(),
        created_at: Utc::now(),
        model: None,
        input_tokens: None,
        output_tokens: None,
        stop_reason: None,
    };
    let user_id = graph.add_message(root_id, user_msg).unwrap();
    assert_eq!(graph.branch_leaf("main"), Some(user_id));

    // add_reply does NOT update the branch leaf.
    let reply = Node::Message {
        id: Uuid::new_v4(),
        role: Role::Assistant,
        content: "Task reply".to_string(),
        created_at: Utc::now(),
        model: None,
        input_tokens: None,
        output_tokens: None,
        stop_reason: None,
    };
    let reply_id = graph.add_reply(user_id, reply).unwrap();

    // Branch leaf should still be user_id, not reply_id.
    assert_eq!(
        graph.branch_leaf("main"),
        Some(user_id),
        "add_reply must not advance the branch leaf"
    );
    // But the reply IS in the graph with a RespondsTo edge.
    assert_eq!(graph.responds_to.get(&reply_id), Some(&user_id));
}

/// Bug: `find_chain_leaf` returns the wrong node because the forward index
/// is not maintained, causing task agents to append to the wrong parent.
#[test]
fn test_find_chain_leaf_walks_forward() {
    let mut graph = ConversationGraph::new("System prompt");
    let root_id = graph.branch_leaf("main").unwrap();

    // Build a chain: root → msg1 → msg2 → msg3 (using add_reply)
    let make_msg = |content: &str| Node::Message {
        id: Uuid::new_v4(),
        role: Role::User,
        content: content.to_string(),
        created_at: Utc::now(),
        model: None,
        input_tokens: None,
        output_tokens: None,
        stop_reason: None,
    };
    let id1 = graph.add_reply(root_id, make_msg("msg1")).unwrap();
    let id2 = graph.add_reply(id1, make_msg("msg2")).unwrap();
    let id3 = graph.add_reply(id2, make_msg("msg3")).unwrap();

    assert_eq!(graph.find_chain_leaf(root_id), id3);
    assert_eq!(graph.find_chain_leaf(id1), id3);
    assert_eq!(graph.find_chain_leaf(id2), id3);
    assert_eq!(graph.find_chain_leaf(id3), id3);
}

/// Bug: `reply_children` index not built during deserialization, causing
/// `find_chain_leaf` to return the root instead of the actual leaf.
#[test]
fn test_reply_children_survives_serialization() {
    let mut graph = ConversationGraph::new("System prompt");
    let root_id = graph.branch_leaf("main").unwrap();

    let msg = Node::Message {
        id: Uuid::new_v4(),
        role: Role::User,
        content: "Hello".to_string(),
        created_at: Utc::now(),
        model: None,
        input_tokens: None,
        output_tokens: None,
        stop_reason: None,
    };
    let msg_id = graph.add_reply(root_id, msg).unwrap();

    // Serialize and deserialize.
    let json = serde_json::to_string(&graph).unwrap();
    let restored: ConversationGraph = serde_json::from_str(&json).unwrap();

    // Forward index should be rebuilt from edges.
    assert_eq!(
        restored.find_chain_leaf(root_id),
        msg_id,
        "reply_children must be rebuilt during deserialization"
    );
}
