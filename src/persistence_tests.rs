use super::*;
use crate::graph::tool_types::{ToolCallArguments, ToolCallStatus};
use crate::graph::{ConversationGraph, EdgeKind, Node};

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

/// `ToolCall` and `ToolResult` nodes must survive the full persistence
/// roundtrip through the `VersionedGraph::V2` envelope.
#[test]
fn test_tool_call_persistence_roundtrip() {
    let tmp = tempfile::tempdir().unwrap();
    let base = tmp.path();

    let mut graph = ConversationGraph::new("System prompt");
    let root_id = graph.branch_leaf("main").unwrap();

    let asst_id = uuid::Uuid::new_v4();
    graph
        .add_message(
            root_id,
            Node::Message {
                id: asst_id,
                role: crate::graph::Role::Assistant,
                content: "I'll look that up.".to_string(),
                created_at: Utc::now(),
                model: Some("test-model".to_string()),
                input_tokens: None,
                output_tokens: None,
            },
        )
        .unwrap();

    let tc_id = uuid::Uuid::new_v4();
    graph.add_node(Node::ToolCall {
        id: tc_id,
        api_tool_use_id: Some("toolu_test_123".to_string()),
        arguments: ToolCallArguments::WebSearch {
            query: "rust serde".to_string(),
        },
        status: ToolCallStatus::Completed,
        parent_message_id: asst_id,
        created_at: Utc::now(),
        completed_at: Some(Utc::now()),
    });
    graph.add_edge(tc_id, asst_id, EdgeKind::Invoked).unwrap();

    let result_id = uuid::Uuid::new_v4();
    graph.add_node(Node::ToolResult {
        id: result_id,
        tool_call_id: tc_id,
        content: crate::graph::tool_types::ToolResultContent::text("Found 42 results"),
        is_error: false,
        created_at: Utc::now(),
    });
    graph
        .add_edge(result_id, tc_id, EdgeKind::Produced)
        .unwrap();

    let metadata = ConversationMetadata {
        id: "tool-call-test".to_string(),
        name: "Tool Call Test".to_string(),
        created_at: Utc::now(),
        last_modified: Utc::now(),
    };

    save_conversation_to(base, "tool-call-test", &metadata, &graph).unwrap();
    let (_, loaded) = load_conversation_from(base, "tool-call-test").unwrap();

    let tool_calls: Vec<_> = loaded
        .nodes_by(|n| matches!(n, Node::ToolCall { .. }))
        .into_iter()
        .collect();
    assert_eq!(tool_calls.len(), 1);
    assert_eq!(tool_calls[0].id(), tc_id);
    assert_eq!(tool_calls[0].content(), "web_search");

    let tool_results: Vec<_> = loaded
        .nodes_by(|n| matches!(n, Node::ToolResult { .. }))
        .into_iter()
        .collect();
    assert_eq!(tool_results.len(), 1);

    // Verify the Invoked edge was rebuilt in the runtime index
    let invoked_targets = loaded.sources_by_edge(asst_id, EdgeKind::Invoked);
    assert_eq!(invoked_targets.len(), 1);
    assert_eq!(invoked_targets[0], tc_id);
}
