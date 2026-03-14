use crate::llm::tool_types::*;

/// Catches malformed tool definitions sent to Anthropic API.
/// The `to_api()` conversion must produce valid JSON Schema structure.
#[test]
fn test_tool_definition_serializes_to_api_schema() {
    let def = ToolDefinition {
        name: "read_file".to_string(),
        description: "Read a file".to_string(),
        input_schema: ToolInputSchema {
            properties: vec![
                SchemaProperty {
                    name: "path".to_string(),
                    property_type: SchemaType::String,
                    description: "File path".to_string(),
                    required: true,
                },
                SchemaProperty {
                    name: "encoding".to_string(),
                    property_type: SchemaType::String,
                    description: "Encoding".to_string(),
                    required: false,
                },
            ],
        },
    };

    let api = def.to_api();
    let json = serde_json::to_string(&api).unwrap();

    assert!(json.contains("\"read_file\""));
    assert!(json.contains("\"path\""));
    assert!(json.contains("\"type\":\"object\""));

    // "path" should be in required, "encoding" should not
    assert!(json.contains("\"required\":[\"path\"]"));
}

/// Catches `ChatMessage::text()` not producing the expected `ChatContent::Text` variant.
#[test]
fn test_chat_message_text_backward_compat() {
    let msg = crate::llm::ChatMessage::text("user", "hello world");
    assert_eq!(msg.role, "user");
    assert!(matches!(msg.content, crate::llm::ChatContent::Text(ref s) if s == "hello world"));
}

/// Catches `ContentBlock` serialization failures for `tool_use`/`tool_result`.
/// Each `ContentBlock` variant must round-trip through serde.
#[test]
fn test_chat_content_blocks_serde_roundtrip() {
    let blocks = vec![
        ContentBlock::Text {
            text: "thinking...".to_string(),
        },
        ContentBlock::ToolUse {
            id: "tu_123".to_string(),
            name: "read_file".to_string(),
            input: RawJson(r#"{"path":"/tmp/x"}"#.to_string()),
        },
        ContentBlock::ToolResult {
            tool_use_id: "tu_123".to_string(),
            content: crate::graph::tool_types::ToolResultContent::text("file contents"),
            is_error: false,
        },
    ];

    let content = ChatContent::Blocks(blocks);
    let json = serde_json::to_string(&content).unwrap();
    let parsed: ChatContent = serde_json::from_str(&json).unwrap();

    // Verify intermediate JSON shape matches Anthropic API format
    assert!(json.contains(r#""type":"tool_result""#));
    assert!(json.contains(r#""tool_use_id":"tu_123""#));
    assert!(json.contains(r#""content":"file contents""#));

    match parsed {
        ChatContent::Blocks(b) => assert_eq!(b.len(), 3),
        ChatContent::Text(_) => panic!("Expected Blocks variant"),
    }
}
