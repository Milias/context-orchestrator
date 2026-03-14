use crate::config::AppConfig;
use crate::llm::error::ApiError;
use crate::llm::retry::{self, RetryConfig};
use crate::llm::tool_types::{ApiToolDefinition, ToolDefinition};
use crate::llm::{ChatConfig, ChatMessage, LlmProvider, StreamChunk};
use async_trait::async_trait;
use futures::stream::Stream;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::pin::Pin;
use std::time::Duration;

use super::sse::sse_to_stream_chunks;

pub struct AnthropicProvider {
    api_key: String,
    base_url: String,
    client: Client,
}

impl AnthropicProvider {
    pub fn from_config(config: &AppConfig) -> anyhow::Result<Self> {
        let api_key = config.api_key()?;
        let base_url = config.anthropic_base_url.trim_end_matches('/').to_string();

        let client = Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .read_timeout(Duration::from_secs(60))
            .build()?;
        Ok(Self {
            api_key,
            base_url,
            client,
        })
    }
}

#[derive(Serialize)]
struct MessagesRequest {
    model: String,
    max_tokens: u32,
    messages: Vec<ChatMessage>,
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    system: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tools: Vec<ApiToolDefinition>,
}

#[derive(Serialize)]
struct CountTokensRequest {
    model: String,
    messages: Vec<ChatMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    system: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tools: Vec<ApiToolDefinition>,
}

#[derive(Deserialize)]
struct CountTokensResponse {
    input_tokens: u32,
}

#[async_trait]
impl LlmProvider for AnthropicProvider {
    async fn chat(
        &self,
        messages: Vec<ChatMessage>,
        config: &ChatConfig,
    ) -> anyhow::Result<Pin<Box<dyn Stream<Item = anyhow::Result<StreamChunk>> + Send>>> {
        let api_tools: Vec<ApiToolDefinition> =
            config.tools.iter().map(ToolDefinition::to_api).collect();

        let body = MessagesRequest {
            model: config.model.clone(),
            max_tokens: config.max_tokens,
            messages,
            stream: true,
            system: config.system_prompt.clone(),
            tools: api_tools,
        };

        let response = self
            .client
            .post(format!("{}/v1/messages", self.base_url))
            .header("x-api-key", &self.api_key)
            .header("authorization", format!("Bearer {}", self.api_key))
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| ApiError::from_reqwest(&e))?;

        if !response.status().is_success() {
            let status = response.status();
            let headers = response.headers().clone();
            let body_text = response.text().await.unwrap_or_default();
            return Err(ApiError::from_response(status, &body_text, &headers).into());
        }

        let byte_stream = response.bytes_stream();
        let chunk_stream = sse_to_stream_chunks(byte_stream);

        Ok(Box::pin(chunk_stream))
    }

    async fn count_tokens(
        &self,
        messages: &[ChatMessage],
        model: &str,
        system_prompt: Option<&str>,
        tools: &[ToolDefinition],
    ) -> anyhow::Result<u32> {
        let api_tools: Vec<ApiToolDefinition> = tools.iter().map(ToolDefinition::to_api).collect();
        let body = CountTokensRequest {
            model: model.to_string(),
            messages: messages.to_vec(),
            system: system_prompt.map(std::string::ToString::to_string),
            tools: api_tools,
        };

        retry::with_retry(&RetryConfig::default(), || {
            let req_body = &body;
            async move {
                let response = self
                    .client
                    .post(format!("{}/v1/messages/count_tokens", self.base_url))
                    .header("x-api-key", &self.api_key)
                    .header("authorization", format!("Bearer {}", self.api_key))
                    .header("anthropic-version", "2023-06-01")
                    .header("content-type", "application/json")
                    .timeout(Duration::from_secs(30))
                    .json(req_body)
                    .send()
                    .await
                    .map_err(|e| ApiError::from_reqwest(&e))?;

                if !response.status().is_success() {
                    let status = response.status();
                    let headers = response.headers().clone();
                    let body_text = response.text().await.unwrap_or_default();
                    return Err(ApiError::from_response(status, &body_text, &headers).into());
                }

                let result: CountTokensResponse = response.json().await?;
                Ok(result.input_tokens)
            }
        })
        .await
    }
}

#[cfg(test)]
#[path = "anthropic_tests.rs"]
mod tests;
