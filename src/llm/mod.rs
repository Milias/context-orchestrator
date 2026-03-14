pub mod anthropic;
pub mod error;
pub mod retry;
pub mod tool_types;

pub use tool_types::{ChatContent, ContentBlock, RawJson, ToolDefinition};

use crate::graph::StopReason;
use async_trait::async_trait;
use futures::stream::Stream;
use serde::{Deserialize, Serialize};
use std::pin::Pin;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: ChatContent,
}

impl ChatMessage {
    /// Convenience constructor for plain-text messages.
    pub fn text(role: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            role: role.into(),
            content: ChatContent::Text(content.into()),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ChatConfig {
    pub model: String,
    pub max_tokens: u32,
    pub system_prompt: Option<String>,
    pub tools: Vec<ToolDefinition>,
}

#[derive(Debug, Clone)]
pub enum StreamChunk {
    TextDelta(String),
    ToolUse {
        id: String,
        name: String,
        input: String,
    },
    Done {
        output_tokens: Option<u32>,
        stop_reason: Option<StopReason>,
    },
    Error(String),
}

#[async_trait]
pub trait LlmProvider: Send + Sync {
    async fn chat(
        &self,
        messages: Vec<ChatMessage>,
        config: &ChatConfig,
    ) -> anyhow::Result<Pin<Box<dyn Stream<Item = anyhow::Result<StreamChunk>> + Send>>>;

    async fn count_tokens(
        &self,
        messages: &[ChatMessage],
        model: &str,
        system_prompt: Option<&str>,
        tools: &[ToolDefinition],
    ) -> anyhow::Result<u32>;
}

#[cfg(test)]
mod tests;

#[cfg(test)]
#[path = "tool_types_tests.rs"]
mod tool_types_tests;
