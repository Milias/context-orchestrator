use super::*;

#[test]
fn test_parse_text_delta() {
    let event = "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"Hello\"}}";
    let mut ot = None;
    let mut sr = None;
    let mut pending = None;
    let result = parse_sse_event(event, &mut ot, &mut sr, &mut pending);
    assert!(matches!(result, Some(Ok(StreamChunk::TextDelta(ref t))) if t == "Hello"));
}

#[test]
fn test_parse_message_start_ignored() {
    let event = "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":25}}}";
    let mut ot = None;
    let mut sr = None;
    let mut pending = None;
    let result = parse_sse_event(event, &mut ot, &mut sr, &mut pending);
    assert!(result.is_none());
}

#[test]
fn test_parse_message_delta_captures_output_tokens() {
    let event = "event: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":100}}";
    let mut ot = None;
    let mut sr = None;
    let mut pending = None;
    let result = parse_sse_event(event, &mut ot, &mut sr, &mut pending);
    assert!(result.is_none());
    assert_eq!(ot, Some(100));
    assert_eq!(sr.as_deref(), Some("end_turn"));
}

#[test]
fn test_parse_message_stop() {
    let event = "event: message_stop\ndata: {\"type\":\"message_stop\"}";
    let mut ot = Some(100);
    let mut sr = Some("end_turn".to_string());
    let mut pending = None;
    let result = parse_sse_event(event, &mut ot, &mut sr, &mut pending);
    assert!(matches!(
        result,
        Some(Ok(StreamChunk::Done {
            output_tokens: Some(100),
            stop_reason: Some(ref r),
        })) if r == "end_turn"
    ));
}

#[test]
fn test_parse_ping_ignored() {
    let event = "event: ping\ndata: {\"type\":\"ping\"}";
    let mut ot = None;
    let mut sr = None;
    let mut pending = None;
    let result = parse_sse_event(event, &mut ot, &mut sr, &mut pending);
    assert!(result.is_none());
}

#[test]
fn test_parse_error_event() {
    let event = "event: error\ndata: {\"type\":\"error\",\"error\":{\"type\":\"overloaded_error\",\"message\":\"Overloaded\"}}";
    let mut ot = None;
    let mut sr = None;
    let mut pending = None;
    let result = parse_sse_event(event, &mut ot, &mut sr, &mut pending);
    assert!(matches!(result, Some(Ok(StreamChunk::Error(ref e))) if e == "Overloaded"));
}

/// Catches `tool_use` SSE events being silently dropped by the parser.
/// A `content_block_start` with type `tool_use` + `input_json_delta` + `content_block_stop`
/// must produce a `StreamChunk::ToolUse`.
#[test]
fn test_parse_tool_use_sse_events() {
    let mut ot = None;
    let mut sr = None;
    let mut pending = None;

    // content_block_start: begin tool_use
    let start = r#"event: content_block_start
data: {"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"toolu_abc","name":"read_file"}}"#;
    let result = parse_sse_event(start, &mut ot, &mut sr, &mut pending);
    assert!(result.is_none());
    assert!(pending.is_some());

    // content_block_delta: accumulate input JSON
    let delta1 = r#"event: content_block_delta
data: {"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{\"path\""}}"#;
    let result = parse_sse_event(delta1, &mut ot, &mut sr, &mut pending);
    assert!(result.is_none());

    let delta2 = r#"event: content_block_delta
data: {"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":":\"/tmp/x\"}"}}"#;
    let result = parse_sse_event(delta2, &mut ot, &mut sr, &mut pending);
    assert!(result.is_none());

    // content_block_stop: emit ToolUse
    let stop = "event: content_block_stop\ndata: {\"type\":\"content_block_stop\",\"index\":1}";
    let result = parse_sse_event(stop, &mut ot, &mut sr, &mut pending);
    match result {
        Some(Ok(StreamChunk::ToolUse { id, name, input })) => {
            assert_eq!(id, "toolu_abc");
            assert_eq!(name, "read_file");
            assert_eq!(input, "{\"path\":\"/tmp/x\"}");
        }
        other => panic!("Expected ToolUse, got: {other:?}"),
    }
    assert!(pending.is_none());
}

#[tokio::test]
async fn test_real_api_call() {
    if std::env::var("ANTHROPIC_AUTH_TOKEN").is_err() && std::env::var("ANTHROPIC_API_KEY").is_err()
    {
        eprintln!("Skipping: no API key set");
        return;
    }
    let app_config = AppConfig::load().unwrap();
    let provider = AnthropicProvider::from_config(&app_config).unwrap();
    let messages = vec![ChatMessage::text("user", "Say hello in exactly 3 words.")];
    let config = ChatConfig {
        model: app_config.anthropic_model.clone(),
        max_tokens: app_config.max_tokens,
        system_prompt: None,
        tools: Vec::new(),
    };
    let mut stream = provider.chat(messages, &config).await.unwrap();

    let mut full_text = String::new();
    while let Some(chunk) = stream.next().await {
        match chunk.unwrap() {
            StreamChunk::TextDelta(t) => full_text.push_str(&t),
            StreamChunk::ToolUse { .. } => {}
            StreamChunk::Done { .. } => break,
            StreamChunk::Error(e) => panic!("Error: {e}"),
        }
    }
    assert!(!full_text.is_empty());
    eprintln!("Response: {full_text}");
}
