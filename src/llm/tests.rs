use super::mock::MockLlmProvider;
use super::{ChatConfig, ChatMessage, LlmProvider, StreamChunk};
use crate::graph::{Role, StopReason};
use futures::StreamExt;

/// Bug: `MockLlmProvider::with_chunks` stream yields wrong chunks or
/// panics — breaks all agent loop tests that depend on mock streaming.
#[tokio::test]
async fn test_mock_provider_streams_configured_chunks() {
    let provider = MockLlmProvider::with_token_count(100).with_chunks(vec![
        StreamChunk::TextDelta("Hello".to_string()),
        StreamChunk::Done {
            output_tokens: Some(5),
            stop_reason: Some(StopReason::EndTurn),
        },
    ]);

    let config = ChatConfig {
        model: "test".to_string(),
        max_tokens: 100,
        system_prompt: None,
        tools: vec![],
    };

    let messages = vec![ChatMessage::text(Role::User, "hi")];
    let mut stream = provider.chat(messages, &config).await.unwrap();

    let first = stream.next().await.unwrap().unwrap();
    assert!(matches!(first, StreamChunk::TextDelta(ref t) if t == "Hello"));

    let second = stream.next().await.unwrap().unwrap();
    assert!(matches!(second, StreamChunk::Done { .. }));

    assert!(stream.next().await.is_none(), "stream should be exhausted");
}

/// Bug: `count_tokens` returns wrong fixed value.
#[tokio::test]
async fn test_mock_provider_returns_fixed_token_count() {
    let provider = MockLlmProvider::with_token_count(42);
    let count = provider
        .count_tokens(&[], "model", None, &[])
        .await
        .unwrap();
    assert_eq!(count, 42);
}
