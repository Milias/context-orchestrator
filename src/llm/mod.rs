pub mod anthropic;

use crate::config::AppConfig;
use async_trait::async_trait;
use futures::stream::Stream;
use serde::{Deserialize, Serialize};
use std::pin::Pin;

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
    Done {
        input_tokens: Option<u32>,
        output_tokens: Option<u32>,
    },
    Error(String),
}

#[allow(dead_code)]
#[async_trait]
pub trait LlmProvider: Send + Sync {
    async fn chat(
        &self,
        messages: Vec<ChatMessage>,
        config: &ChatConfig,
    ) -> anyhow::Result<Pin<Box<dyn Stream<Item = anyhow::Result<StreamChunk>> + Send>>>;

    fn name(&self) -> &str;
}
