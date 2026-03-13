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

    // Check migrated file has current version
    let migrated_data = std::fs::read_to_string(&graph_path).unwrap();
    assert_eq!(detect_version(&migrated_data), 3);
}

#[test]
fn test_v3_load_no_migration() {
    let tmp = tempfile::tempdir().unwrap();
    let graph_path = tmp.path().join("graph.json");

    // Create and save a current-format graph
    let graph = ConversationGraph::new("V3 prompt");
    let json = to_versioned_json(&graph).unwrap();
    std::fs::write(&graph_path, &json).unwrap();

    // Load -- should not create a backup
    let loaded = load_and_migrate(&graph_path).unwrap();
    assert_eq!(loaded.active_branch(), "main");
    let history = loaded.get_branch_history("main").unwrap();
    assert_eq!(history[0].content(), "V3 prompt");

    // No backup should exist
    let backup_path = tmp.path().join("graph.v3.json.bak");
    assert!(!backup_path.exists());
}

#[test]
fn test_detect_version_v3() {
    let v3_json = r#"{"version":"3","nodes":{},"edges":[],"branches":{"main":"00000000-0000-0000-0000-000000000000"},"active_branch":"main","history":{}}"#;
    assert_eq!(detect_version(v3_json), 3);
}

/// V2 graphs migrated to V3 get empty history and a backup file.
#[test]
fn test_v2_to_v3_migration() {
    let tmp = tempfile::tempdir().unwrap();
    let graph_path = tmp.path().join("graph.json");

    // Write a V2-format graph
    let id = Uuid::new_v4();
    let v2 = V2Graph {
        nodes: {
            let mut m = HashMap::new();
            m.insert(
                id,
                crate::graph::Node::SystemDirective {
                    id,
                    content: "V2 system".to_string(),
                    created_at: Utc::now(),
                },
            );
            m
        },
        edges: Vec::new(),
        branches: {
            let mut b = HashMap::new();
            b.insert("main".to_string(), id);
            b
        },
        active_branch: "main".to_string(),
    };
    let v2_json = serde_json::to_string_pretty(&VersionedGraph::V2(v2)).unwrap();
    std::fs::write(&graph_path, &v2_json).unwrap();

    let loaded = load_and_migrate(&graph_path).unwrap();
    assert_eq!(loaded.active_branch(), "main");
    assert!(loaded.node_history(id).is_empty());

    // Backup created
    assert!(tmp.path().join("graph.v2.json.bak").exists());

    // Migrated file is V3
    let migrated_data = std::fs::read_to_string(&graph_path).unwrap();
    assert_eq!(detect_version(&migrated_data), 3);
}

#[test]
fn test_to_versioned_json_includes_version() {
    let graph = ConversationGraph::new("Test");
    let json = to_versioned_json(&graph).unwrap();
    assert_eq!(detect_version(&json), 3);
}
