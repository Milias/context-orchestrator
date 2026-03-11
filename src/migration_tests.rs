use super::*;
use crate::graph::ConversationGraph;

#[test]
fn test_detect_version_v1_no_field() {
    let v1_json = r#"{"nodes":{},"edges":{},"branches":{"main":"00000000-0000-0000-0000-000000000000"},"active_branch":"main"}"#;
    assert_eq!(detect_version(v1_json), 1);
}

#[test]
fn test_detect_version_v2() {
    let v2_json = r#"{"version":"2","nodes":{},"edges":[],"branches":{"main":"00000000-0000-0000-0000-000000000000"},"active_branch":"main"}"#;
    assert_eq!(detect_version(v2_json), 2);
}

#[test]
fn test_v1_migration_roundtrip() {
    let tmp = tempfile::tempdir().unwrap();
    let graph_path = tmp.path().join("graph.json");

    // Create a V1-format graph (the old format)
    let graph = ConversationGraph::new("Test prompt");
    let root_id = graph.branch_leaf("main").unwrap();

    // Manually build V1 JSON (old format with HashMap edges)
    let v1 = V1Graph {
        nodes: {
            let mut m = HashMap::new();
            m.insert(
                root_id,
                V1Node::SystemDirective {
                    id: root_id,
                    content: "Test prompt".to_string(),
                    created_at: Utc::now(),
                },
            );
            m
        },
        edges: HashMap::new(),
        branches: {
            let mut b = HashMap::new();
            b.insert("main".to_string(), root_id);
            b
        },
        active_branch: "main".to_string(),
    };

    // Write V1 format (no version field)
    std::fs::write(&graph_path, serde_json::to_string_pretty(&v1).unwrap()).unwrap();

    // Load and migrate
    let loaded = load_and_migrate(&graph_path).unwrap();
    assert_eq!(loaded.active_branch(), "main");
    let history = loaded.get_branch_history("main").unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].content(), "Test prompt");

    // Check backup was created
    let backup_path = tmp.path().join("graph.v1.json.bak");
    assert!(backup_path.exists());

    // Check migrated file has version field
    let migrated_data = std::fs::read_to_string(&graph_path).unwrap();
    assert_eq!(detect_version(&migrated_data), 2);
}

#[test]
fn test_v2_load_no_migration() {
    let tmp = tempfile::tempdir().unwrap();
    let graph_path = tmp.path().join("graph.json");

    // Create and save a current-format graph
    let graph = ConversationGraph::new("V2 prompt");
    let json = to_versioned_json(&graph).unwrap();
    std::fs::write(&graph_path, &json).unwrap();

    // Load -- should not create a backup
    let loaded = load_and_migrate(&graph_path).unwrap();
    assert_eq!(loaded.active_branch(), "main");
    let history = loaded.get_branch_history("main").unwrap();
    assert_eq!(history[0].content(), "V2 prompt");

    // No backup should exist
    let backup_path = tmp.path().join("graph.v2.json.bak");
    assert!(!backup_path.exists());
}

#[test]
fn test_to_versioned_json_includes_version() {
    let graph = ConversationGraph::new("Test");
    let json = to_versioned_json(&graph).unwrap();
    assert_eq!(detect_version(&json), 2);
}
