use super::*;
use crate::graph::tool_types::{ToolCallArguments, ToolCallStatus};

/// Catches serialization failures when `ToolCall` nodes are saved/loaded.
/// `ToolCall` has nested enums (`ToolCallArguments`, `ToolCallStatus`) that
/// must round-trip through serde without data loss.
#[test]
fn test_tool_call_serde_roundtrip() {
    let mut graph = ConversationGraph::new("system");
    let root_id = graph.branch_leaf("main").unwrap();

    let asst_id = Uuid::new_v4();
    let asst = Node::Message {
        id: asst_id,
        role: Role::Assistant,
        content: "I'll search for that.".to_string(),
        created_at: Utc::now(),
        model: Some("test".to_string()),
        input_tokens: None,
        output_tokens: None,
    };
    graph.add_message(root_id, asst).unwrap();

    let tc_id = Uuid::new_v4();
    let tool_call = Node::ToolCall {
        id: tc_id,
        api_tool_use_id: None,
        arguments: ToolCallArguments::WebSearch {
            query: "rust serde tagged".to_string(),
        },
        status: ToolCallStatus::Completed,
        parent_message_id: asst_id,
        created_at: Utc::now(),
        completed_at: Some(Utc::now()),
    };
    graph.add_node(tool_call);
    graph.add_edge(tc_id, asst_id, EdgeKind::Invoked).unwrap();

    let json = serde_json::to_string(&graph).unwrap();
    let restored: ConversationGraph = serde_json::from_str(&json).unwrap();

    let restored_node = restored
        .nodes_by(|n| matches!(n, Node::ToolCall { .. }))
        .into_iter()
        .next()
        .expect("ToolCall node missing after deserialization");
    assert_eq!(restored_node.id(), tc_id);
    assert_eq!(restored_node.content(), "web_search");
}

/// Catches broken `Produced` edge creation between `ToolCall` and `ToolResult`.
/// If the `Produced` edge fails to link, the result is orphaned.
#[test]
fn test_tool_result_linked_to_tool_call() {
    let mut graph = ConversationGraph::new("system");

    let tc_id = Uuid::new_v4();
    let tool_call = Node::ToolCall {
        id: tc_id,
        api_tool_use_id: None,
        arguments: ToolCallArguments::ReadFile {
            path: "/tmp/test".to_string(),
        },
        status: ToolCallStatus::Completed,
        parent_message_id: Uuid::new_v4(),
        created_at: Utc::now(),
        completed_at: Some(Utc::now()),
    };
    graph.add_node(tool_call);

    let result_id = Uuid::new_v4();
    let result = Node::ToolResult {
        id: result_id,
        tool_call_id: tc_id,
        content: ToolResultContent::text("file contents"),
        is_error: false,
        created_at: Utc::now(),
    };
    graph.add_node(result);
    graph
        .add_edge(result_id, tc_id, EdgeKind::Produced)
        .unwrap();

    let produced_edges: Vec<_> = graph
        .edges
        .iter()
        .filter(|e| e.kind == EdgeKind::Produced)
        .collect();
    assert_eq!(produced_edges.len(), 1);
    assert_eq!(produced_edges[0].from, result_id);
    assert_eq!(produced_edges[0].to, tc_id);
}

/// Catches status update failing on missing node or wrong variant.
/// `update_tool_call_status` must only work on `ToolCall` nodes and reject others.
#[test]
fn test_update_tool_call_status() {
    let mut graph = ConversationGraph::new("system");

    let tc_id = Uuid::new_v4();
    let tool_call = Node::ToolCall {
        id: tc_id,
        api_tool_use_id: None,
        arguments: ToolCallArguments::Plan {
            raw_input: "fix bug".to_string(),
            description: None,
        },
        status: ToolCallStatus::Pending,
        parent_message_id: Uuid::new_v4(),
        created_at: Utc::now(),
        completed_at: None,
    };
    graph.add_node(tool_call);

    graph
        .update_tool_call_status(tc_id, ToolCallStatus::Running, None)
        .unwrap();

    let node = graph.nodes_by(|n| n.id() == tc_id).pop().unwrap();
    match node {
        Node::ToolCall { status, .. } => assert_eq!(*status, ToolCallStatus::Running),
        _ => panic!("Expected ToolCall"),
    }

    // Updating a non-existent node should fail
    let missing = Uuid::new_v4();
    assert!(graph
        .update_tool_call_status(missing, ToolCallStatus::Failed, None)
        .is_err());

    // Updating a non-ToolCall node should fail
    let root_id = graph.branch_leaf("main").unwrap();
    assert!(graph
        .update_tool_call_status(root_id, ToolCallStatus::Failed, None)
        .is_err());
}

/// Catches `invoked_by` index not rebuilt on deserialization.
/// After round-tripping through serde, `Invoked` edges must still populate the runtime index.
#[test]
fn test_invoked_edge_rebuilds_runtime_index() {
    let mut graph = ConversationGraph::new("system");
    let root_id = graph.branch_leaf("main").unwrap();

    let asst_id = Uuid::new_v4();
    let asst = Node::Message {
        id: asst_id,
        role: Role::Assistant,
        content: "ok".to_string(),
        created_at: Utc::now(),
        model: None,
        input_tokens: None,
        output_tokens: None,
    };
    graph.add_message(root_id, asst).unwrap();

    let tc_id = Uuid::new_v4();
    let tool_call = Node::ToolCall {
        id: tc_id,
        api_tool_use_id: None,
        arguments: ToolCallArguments::Unknown {
            tool_name: "custom".to_string(),
            raw_json: "{}".to_string(),
        },
        status: ToolCallStatus::Pending,
        parent_message_id: asst_id,
        created_at: Utc::now(),
        completed_at: None,
    };
    graph.add_node(tool_call);
    graph.add_edge(tc_id, asst_id, EdgeKind::Invoked).unwrap();

    assert_eq!(graph.invoked_by.get(&tc_id), Some(&asst_id));

    let json = serde_json::to_string(&graph).unwrap();
    let restored: ConversationGraph = serde_json::from_str(&json).unwrap();
    assert_eq!(restored.invoked_by.get(&tc_id), Some(&asst_id));
}

/// Catches `ToolCallArguments` tag discrimination failures across variants.
/// Each variant must serialize with a unique `tool_type` discriminant.
#[test]
fn test_tool_call_arguments_tagged_union_serde() {
    let variants = vec![
        ToolCallArguments::Plan {
            raw_input: "fix".to_string(),
            description: Some("desc".to_string()),
        },
        ToolCallArguments::ReadFile {
            path: "/a".to_string(),
        },
        ToolCallArguments::WriteFile {
            path: "/b".to_string(),
            content: "data".to_string(),
        },
        ToolCallArguments::WebSearch {
            query: "q".to_string(),
        },
        ToolCallArguments::Unknown {
            tool_name: "mcp_thing".to_string(),
            raw_json: r#"{"x":1}"#.to_string(),
        },
    ];

    for variant in &variants {
        let json = serde_json::to_string(variant).unwrap();
        let parsed: ToolCallArguments = serde_json::from_str(&json).unwrap();
        assert_eq!(
            variant.tool_name(),
            parsed.tool_name(),
            "Round-trip failed for {}",
            variant.tool_name()
        );
    }
}

/// Verifies the graph query pattern used by `build_assistant_message_with_tools`:
/// given an assistant message, find its `ToolCall` nodes via `Invoked` edges,
/// then find the `ToolResult` for each via `Produced` edges.
#[test]
fn test_tool_call_provenance_chain_query() {
    let mut graph = ConversationGraph::new("system");
    let root_id = graph.branch_leaf("main").unwrap();

    let asst_id = Uuid::new_v4();
    graph
        .add_message(
            root_id,
            Node::Message {
                id: asst_id,
                role: Role::Assistant,
                content: "Let me search.".to_string(),
                created_at: Utc::now(),
                model: None,
                input_tokens: None,
                output_tokens: None,
            },
        )
        .unwrap();

    // Two tool calls from the same assistant message
    let tc1_id = Uuid::new_v4();
    graph.add_node(Node::ToolCall {
        id: tc1_id,
        api_tool_use_id: Some("toolu_aaa".to_string()),
        arguments: ToolCallArguments::WebSearch {
            query: "q1".to_string(),
        },
        status: ToolCallStatus::Completed,
        parent_message_id: asst_id,
        created_at: Utc::now(),
        completed_at: Some(Utc::now()),
    });
    graph.add_edge(tc1_id, asst_id, EdgeKind::Invoked).unwrap();

    let tc2_id = Uuid::new_v4();
    graph.add_node(Node::ToolCall {
        id: tc2_id,
        api_tool_use_id: None,
        arguments: ToolCallArguments::ReadFile {
            path: "/tmp/f".to_string(),
        },
        status: ToolCallStatus::Completed,
        parent_message_id: asst_id,
        created_at: Utc::now(),
        completed_at: Some(Utc::now()),
    });
    graph.add_edge(tc2_id, asst_id, EdgeKind::Invoked).unwrap();

    // Results for each
    let r1_id = Uuid::new_v4();
    graph.add_node(Node::ToolResult {
        id: r1_id,
        tool_call_id: tc1_id,
        content: ToolResultContent::text("result 1"),
        is_error: false,
        created_at: Utc::now(),
    });
    graph.add_edge(r1_id, tc1_id, EdgeKind::Produced).unwrap();

    let r2_id = Uuid::new_v4();
    graph.add_node(Node::ToolResult {
        id: r2_id,
        tool_call_id: tc2_id,
        content: ToolResultContent::text("result 2"),
        is_error: false,
        created_at: Utc::now(),
    });
    graph.add_edge(r2_id, tc2_id, EdgeKind::Produced).unwrap();

    // Query pattern: find tool calls for the assistant message
    let tc_ids = graph.sources_by_edge(asst_id, EdgeKind::Invoked);
    assert_eq!(tc_ids.len(), 2);

    // For each tool call, find its result
    for tc_id in &tc_ids {
        let result_ids = graph.sources_by_edge(*tc_id, EdgeKind::Produced);
        assert_eq!(
            result_ids.len(),
            1,
            "each tool call should have exactly one result"
        );
        let result = graph.node(result_ids[0]).unwrap();
        assert!(matches!(result, Node::ToolResult { .. }));
    }

    // Verify API ID fallback logic
    let tc1 = graph.node(tc1_id).unwrap();
    if let Node::ToolCall {
        api_tool_use_id,
        id,
        ..
    } = tc1
    {
        let use_id = api_tool_use_id.clone().unwrap_or_else(|| id.to_string());
        assert_eq!(use_id, "toolu_aaa");
    }
    let tc2 = graph.node(tc2_id).unwrap();
    if let Node::ToolCall {
        api_tool_use_id,
        id,
        ..
    } = tc2
    {
        let use_id = api_tool_use_id.clone().unwrap_or_else(|| id.to_string());
        assert_eq!(use_id, tc2_id.to_string(), "should fall back to UUID");
    }
}

/// Catches `to_input_json` producing invalid JSON or including the serde tag.
/// The Anthropic API expects raw input fields without the `tool_type` discriminant.
#[test]
fn test_to_input_json_strips_tag() {
    let read = ToolCallArguments::ReadFile {
        path: "/tmp/test.rs".to_string(),
    };
    let json = read.to_input_json();
    let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert!(parsed.get("tool_type").is_none(), "tag should be stripped");
    assert_eq!(
        parsed.get("path").unwrap().as_str().unwrap(),
        "/tmp/test.rs"
    );

    let unknown = ToolCallArguments::Unknown {
        tool_name: "mcp_tool".to_string(),
        raw_json: r#"{"key":"val"}"#.to_string(),
    };
    assert_eq!(unknown.to_input_json(), r#"{"key":"val"}"#);

    let plan = ToolCallArguments::Plan {
        raw_input: "fix the bug".to_string(),
        description: Some("plan desc".to_string()),
    };
    let plan_json = plan.to_input_json();
    let plan_parsed: serde_json::Value = serde_json::from_str(&plan_json).unwrap();
    assert!(plan_parsed.get("tool_type").is_none());
    assert_eq!(
        plan_parsed.get("raw_input").unwrap().as_str().unwrap(),
        "fix the bug"
    );

    let write = ToolCallArguments::WriteFile {
        path: "/out.txt".to_string(),
        content: "hello".to_string(),
    };
    let write_json = write.to_input_json();
    let write_parsed: serde_json::Value = serde_json::from_str(&write_json).unwrap();
    assert!(write_parsed.get("tool_type").is_none());
    assert_eq!(
        write_parsed.get("path").unwrap().as_str().unwrap(),
        "/out.txt"
    );
    assert_eq!(
        write_parsed.get("content").unwrap().as_str().unwrap(),
        "hello"
    );

    let search = ToolCallArguments::WebSearch {
        query: "rust serde".to_string(),
    };
    let search_json = search.to_input_json();
    let search_parsed: serde_json::Value = serde_json::from_str(&search_json).unwrap();
    assert!(search_parsed.get("tool_type").is_none());
    assert_eq!(
        search_parsed.get("query").unwrap().as_str().unwrap(),
        "rust serde"
    );

    // H1: invalid JSON in Unknown falls back to empty object
    let bad_json = ToolCallArguments::Unknown {
        tool_name: "broken".to_string(),
        raw_json: "not valid json {{{".to_string(),
    };
    assert_eq!(bad_json.to_input_json(), "{}");
}

#[test]
fn test_tool_result_content_text_serde_roundtrip() {
    let content = ToolResultContent::text("hello");
    let json = serde_json::to_string(&content).unwrap();
    assert_eq!(json, r#""hello""#);
    let parsed: ToolResultContent = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed.text_content(), "hello");
    assert!(!parsed.has_images());
}

#[test]
fn test_tool_result_content_blocks_serde_roundtrip() {
    use crate::graph::tool_types::{ImageSource, ToolResultContentBlock};

    let content = ToolResultContent::Blocks(vec![
        ToolResultContentBlock::Text {
            text: "here is the image".to_string(),
        },
        ToolResultContentBlock::Image {
            source: ImageSource::Base64 {
                media_type: "image/png".to_string(),
                data: "iVBOR".to_string(),
            },
        },
    ]);
    let json = serde_json::to_string(&content).unwrap();
    let parsed: ToolResultContent = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed.text_content(), "here is the image");
    assert!(parsed.has_images());
}

#[test]
fn test_tool_result_content_backward_compat_v2() {
    let old_json = r#""file contents here""#;
    let parsed: ToolResultContent = serde_json::from_str(old_json).unwrap();
    assert_eq!(parsed.text_content(), "file contents here");
    assert!(!parsed.has_images());
}

#[test]
fn test_tool_result_content_char_len() {
    use crate::graph::tool_types::{ImageSource, ToolResultContentBlock};

    let text = ToolResultContent::text("hello");
    assert_eq!(text.char_len(), 5);

    let blocks = ToolResultContent::Blocks(vec![
        ToolResultContentBlock::Text {
            text: "abc".to_string(),
        },
        ToolResultContentBlock::Image {
            source: ImageSource::Base64 {
                media_type: "image/png".to_string(),
                data: "AAAA".to_string(),
            },
        },
    ]);
    assert_eq!(blocks.char_len(), 7); // 3 + 4
}
