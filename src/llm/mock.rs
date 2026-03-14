//! Mock `LlmProvider` for testing context pipelines and agent loops.
//!
//! Returns pre-configured token counts and (optionally) stream chunks.
//! All fields have sensible defaults via the builder.

use super::{ChatConfig, ChatMessage, LlmProvider, StreamChunk, ToolDefinition};
use async_trait::async_trait;
use futures::stream;
use std::pin::Pin;

/// A mock LLM provider that returns fixed token counts and optional stream chunks.
pub struct MockLlmProvider {
    /// Fixed token count returned by `count_tokens`.
    token_count: u32,
    /// Optional stream chunks returned by `chat()`. When `None`, returns an empty stream.
    chunks: Option<Vec<StreamChunk>>,
}

impl MockLlmProvider {
    /// Create a mock that returns the given fixed token count and an empty stream.
    pub fn with_token_count(token_count: u32) -> Self {
        Self {
            token_count,
            chunks: None,
        }
    }

    /// Configure the stream chunks returned by `chat()`.
    /// Each call to `chat()` yields these chunks in order.
    pub fn with_chunks(mut self, chunks: Vec<StreamChunk>) -> Self {
        self.chunks = Some(chunks);
        self
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
        match &self.chunks {
            Some(chunks) => {
                let items: Vec<anyhow::Result<StreamChunk>> =
                    chunks.iter().cloned().map(Ok).collect();
                Ok(Box::pin(stream::iter(items)))
            }
            None => Ok(Box::pin(stream::empty())),
        }
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
