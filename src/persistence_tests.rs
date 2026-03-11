use super::*;
use crate::graph::ConversationGraph;

#[test]
fn test_save_and_load_roundtrip() {
    let tmp = tempfile::tempdir().unwrap();
    let base = tmp.path();

    let graph = ConversationGraph::new("Test prompt");
    let metadata = ConversationMetadata {
        id: "test-conv-1".to_string(),
        name: "Test Conversation".to_string(),
        created_at: Utc::now(),
        last_modified: Utc::now(),
    };

    save_conversation_to(base, "test-conv-1", &metadata, &graph).unwrap();
    let (loaded_meta, loaded_graph) = load_conversation_from(base, "test-conv-1").unwrap();

    assert_eq!(loaded_meta.id, "test-conv-1");
    assert_eq!(loaded_meta.name, "Test Conversation");

    let orig_history = graph.get_branch_history("main").unwrap();
    let loaded_history = loaded_graph.get_branch_history("main").unwrap();
    assert_eq!(orig_history.len(), loaded_history.len());
    assert_eq!(orig_history[0].content(), loaded_history[0].content());
}

#[test]
fn test_list_conversations() {
    let tmp = tempfile::tempdir().unwrap();
    let base = tmp.path();

    for i in 0..3 {
        let graph = ConversationGraph::new("Prompt");
        let metadata = ConversationMetadata {
            id: format!("list-test-conv-{i}"),
            name: format!("Conversation {i}"),
            created_at: Utc::now(),
            last_modified: Utc::now(),
        };
        save_conversation_to(base, &metadata.id, &metadata, &graph).unwrap();
    }

    let list = list_conversations_in(base).unwrap();
    assert_eq!(list.len(), 3);
}

#[test]
fn test_load_nonexistent_errors() {
    let tmp = tempfile::tempdir().unwrap();
    assert!(load_conversation_from(tmp.path(), "does-not-exist").is_err());
}
