//! Mock `LlmProvider` for testing context pipelines and agent loops.
//!
//! Returns pre-configured token counts and (optionally) stream chunks.
//! All fields have sensible defaults via the builder.

use super::{ChatConfig, ChatMessage, LlmProvider, StreamChunk, ToolDefinition};
use async_trait::async_trait;
use futures::stream;
use std::pin::Pin;

/// A mock LLM provider that returns fixed token counts.
/// Extend with `chunks` to also mock streaming responses.
pub struct MockLlmProvider {
    /// Fixed token count returned by `count_tokens`.
    token_count: u32,
}

impl MockLlmProvider {
    /// Create a mock that returns the given fixed token count.
    pub fn with_token_count(token_count: u32) -> Self {
        Self { token_count }
    }
}

#[async_trait]
impl LlmProvider for MockLlmProvider {
    async fn chat(
        &self,
        _messages: Vec<ChatMessage>,
        _config: &ChatConfig,
    ) -> anyhow::Result<
        Pin<Box<dyn futures::stream::Stream<Item = anyhow::Result<StreamChunk>> + Send>>,
    > {
        // Return an empty stream — tests that need streaming should extend this.
        Ok(Box::pin(stream::empty()))
    }

    async fn count_tokens(
        &self,
        _messages: &[ChatMessage],
        _model: &str,
        _system_prompt: Option<&str>,
        _tools: &[ToolDefinition],
    ) -> anyhow::Result<u32> {
        Ok(self.token_count)
    }
}
