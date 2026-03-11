use super::*;

#[test]
fn test_parse_text_delta() {
    let event = "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"Hello\"}}";
    let mut ot = None;
    let result = parse_sse_event(event, &mut ot);
    assert!(matches!(result, Some(Ok(StreamChunk::TextDelta(ref t))) if t == "Hello"));
}

#[test]
fn test_parse_message_start_ignored() {
    let event = "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":25}}}";
    let mut ot = None;
    let result = parse_sse_event(event, &mut ot);
    assert!(result.is_none());
}

#[test]
fn test_parse_message_delta_captures_output_tokens() {
    let event = "event: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":100}}";
    let mut ot = None;
    let result = parse_sse_event(event, &mut ot);
    assert!(result.is_none());
    assert_eq!(ot, Some(100));
}

#[test]
fn test_parse_message_stop() {
    let event = "event: message_stop\ndata: {\"type\":\"message_stop\"}";
    let mut ot = Some(100);
    let result = parse_sse_event(event, &mut ot);
    assert!(matches!(
        result,
        Some(Ok(StreamChunk::Done {
            output_tokens: Some(100)
        }))
    ));
}

#[test]
fn test_parse_ping_ignored() {
    let event = "event: ping\ndata: {\"type\":\"ping\"}";
    let mut ot = None;
    let result = parse_sse_event(event, &mut ot);
    assert!(result.is_none());
}

#[test]
fn test_parse_error_event() {
    let event = "event: error\ndata: {\"type\":\"error\",\"error\":{\"type\":\"overloaded_error\",\"message\":\"Overloaded\"}}";
    let mut ot = None;
    let result = parse_sse_event(event, &mut ot);
    assert!(matches!(result, Some(Ok(StreamChunk::Error(ref e))) if e == "Overloaded"));
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
    let messages = vec![ChatMessage {
        role: "user".to_string(),
        content: "Say hello in exactly 3 words.".to_string(),
    }];
    let config = ChatConfig::from_app_config(&app_config);
    let mut stream = provider.chat(messages, &config).await.unwrap();

    let mut full_text = String::new();
    while let Some(chunk) = stream.next().await {
        match chunk.unwrap() {
            StreamChunk::TextDelta(t) => full_text.push_str(&t),
            StreamChunk::Done { .. } => break,
            StreamChunk::Error(e) => panic!("Error: {e}"),
        }
    }
    assert!(!full_text.is_empty());
    eprintln!("Response: {full_text}");
}
