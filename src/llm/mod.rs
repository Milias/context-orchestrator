pub mod anthropic;

use crate::config::AppConfig;
use async_trait::async_trait;
use futures::stream::Stream;
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use std::pin::Pin;
use tokio::sync::Semaphore;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Clone)]
pub struct ChatConfig {
    pub model: String,
    pub max_tokens: u32,
    pub system_prompt: Option<String>,
}

impl ChatConfig {
    pub fn from_app_config(config: &AppConfig) -> Self {
        Self {
            model: config.anthropic_model.clone(),
            max_tokens: config.max_tokens,
            system_prompt: None,
        }
    }
}

#[derive(Debug, Clone)]
pub enum StreamChunk {
    TextDelta(String),
    Done { output_tokens: Option<u32> },
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
    ) -> anyhow::Result<u32>;
}

#[derive(Debug, Clone)]
pub struct BackgroundLlmConfig {
    pub model: String,
    pub max_tokens: u32,
}

impl BackgroundLlmConfig {
    pub fn from_app_config(config: &AppConfig) -> Self {
        Self {
            model: config.background_model.clone(),
            max_tokens: config.background_max_tokens,
        }
    }

    pub fn to_chat_config(&self, system_prompt: Option<String>) -> ChatConfig {
        ChatConfig {
            model: self.model.clone(),
            max_tokens: self.max_tokens,
            system_prompt,
        }
    }
}

#[derive(Debug)]
pub struct BackgroundLlmResponse {
    pub content: String,
}

/// Non-streaming LLM call for background tasks. Acquires a semaphore permit
/// to limit concurrent background calls (main conversation bypasses this).
pub async fn background_llm_call(
    provider: &dyn LlmProvider,
    messages: Vec<ChatMessage>,
    config: &ChatConfig,
    semaphore: &Semaphore,
) -> anyhow::Result<BackgroundLlmResponse> {
    let _permit = semaphore.acquire().await?;

    let mut stream = provider.chat(messages, config).await?;
    let mut full_text = String::new();

    while let Some(chunk) = stream.next().await {
        match chunk? {
            StreamChunk::TextDelta(t) => full_text.push_str(&t),
            StreamChunk::Done { .. } => break,
            StreamChunk::Error(e) => anyhow::bail!("LLM error: {e}"),
        }
    }

    Ok(BackgroundLlmResponse {
        content: strip_json_fences(&full_text),
    })
}

/// Strip markdown code fences from LLM responses that wrap JSON output.
pub(crate) fn strip_json_fences(text: &str) -> String {
    let trimmed = text.trim();
    if let Some(rest) = trimmed.strip_prefix("```json") {
        if let Some(content) = rest.strip_suffix("```") {
            return content.trim().to_string();
        }
    }
    if let Some(rest) = trimmed.strip_prefix("```") {
        if let Some(content) = rest.strip_suffix("```") {
            return content.trim().to_string();
        }
    }
    trimmed.to_string()
}

#[cfg(test)]
mod tests;
