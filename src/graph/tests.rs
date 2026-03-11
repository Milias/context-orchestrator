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
        status: WorkItemStatus::Todo,
        description: None,
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
        status: WorkItemStatus::Active,
        description: None,
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
fn test_upsert_node() {
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
    graph.upsert_node(task);
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

    let updated = Node::BackgroundTask {
        id,
        kind: BackgroundTaskKind::GitIndex,
        status: TaskStatus::Completed,
        description: "Indexing...".to_string(),
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };
    graph.upsert_node(updated);
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
fn test_branch_names() {
    let graph = ConversationGraph::new("System prompt");
    let names = graph.branch_names();
    assert_eq!(names.len(), 1);
    assert!(names.contains(&"main"));
}
