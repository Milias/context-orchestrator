use crate::config::AppConfig;
use crate::graph::StopReason;
use crate::llm::error::ApiError;
use crate::llm::retry::{self, RetryConfig};
use crate::llm::tool_types::{ApiToolDefinition, ToolDefinition};
use crate::llm::{ChatConfig, ChatMessage, LlmProvider, StreamChunk};
use async_trait::async_trait;
use futures::stream::{Stream, StreamExt};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::pin::Pin;
use std::time::Duration;

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

// ── SSE parsing ─────────────────────────────────────────────────────

struct SseState<S> {
    stream: Pin<Box<S>>,
    buffer: String,
    output_tokens: Option<u32>,
    stop_reason: Option<StopReason>,
    /// Pending `tool_use` block being accumulated across SSE events.
    pending_tool_use: Option<PendingToolUse>,
}

struct PendingToolUse {
    id: String,
    name: String,
    input_json: String,
}

fn sse_to_stream_chunks(
    byte_stream: impl Stream<Item = Result<bytes::Bytes, reqwest::Error>> + Send + 'static,
) -> impl Stream<Item = anyhow::Result<StreamChunk>> + Send + 'static {
    futures::stream::unfold(
        SseState {
            stream: Box::pin(byte_stream),
            buffer: String::new(),
            output_tokens: None,
            stop_reason: None,
            pending_tool_use: None,
        },
        |mut state| async move {
            loop {
                // Check if buffer contains a complete event
                if let Some(pos) = state.buffer.find("\n\n") {
                    let event_text = state.buffer[..pos].to_string();
                    state.buffer = state.buffer[pos + 2..].to_string();

                    if let Some(chunk) = parse_sse_event(
                        &event_text,
                        &mut state.output_tokens,
                        &mut state.stop_reason,
                        &mut state.pending_tool_use,
                    ) {
                        return Some((chunk, state));
                    }
                    continue;
                }

                // Need more data
                match state.stream.next().await {
                    Some(Ok(bytes)) => {
                        state.buffer.push_str(&String::from_utf8_lossy(&bytes));
                    }
                    Some(Err(e)) => {
                        return Some((Err(ApiError::from_reqwest(&e).into()), state));
                    }
                    None => {
                        return None;
                    }
                }
            }
        },
    )
}

#[derive(Deserialize)]
struct ContentBlockStartEvent {
    content_block: Option<ContentBlockInfo>,
}

#[derive(Deserialize)]
struct ContentBlockInfo {
    #[serde(rename = "type")]
    block_type: Option<String>,
    id: Option<String>,
    name: Option<String>,
}

#[derive(Deserialize)]
struct ContentBlockDeltaEvent {
    delta: Option<DeltaPayload>,
}

#[derive(Deserialize)]
struct DeltaPayload {
    #[serde(rename = "type")]
    delta_type: Option<String>,
    text: Option<String>,
    partial_json: Option<String>,
}

#[derive(Deserialize)]
struct MessageDeltaEvent {
    usage: Option<UsagePayload>,
    delta: Option<MessageDeltaPayload>,
}

#[derive(Deserialize)]
struct MessageDeltaPayload {
    stop_reason: Option<String>,
}

#[derive(Deserialize)]
struct UsagePayload {
    output_tokens: Option<u32>,
}

#[derive(Deserialize)]
struct ErrorEvent {
    error: Option<ErrorPayload>,
}

#[derive(Deserialize)]
struct ErrorPayload {
    message: Option<String>,
}

fn parse_sse_event(
    event_text: &str,
    output_tokens: &mut Option<u32>,
    stop_reason: &mut Option<StopReason>,
    pending_tool_use: &mut Option<PendingToolUse>,
) -> Option<anyhow::Result<StreamChunk>> {
    let mut event_type = "";
    let mut data = "";

    for line in event_text.lines() {
        if let Some(val) = line.strip_prefix("event: ") {
            event_type = val.trim();
        } else if let Some(val) = line.strip_prefix("data: ") {
            data = val.trim();
        }
    }

    if data.is_empty() {
        return None;
    }

    match event_type {
        "content_block_start" => {
            let event: ContentBlockStartEvent = match serde_json::from_str(data) {
                Ok(v) => v,
                Err(e) => return Some(Err(e.into())),
            };
            if let Some(block) = event.content_block {
                if block.block_type.as_deref() == Some("tool_use") {
                    *pending_tool_use = Some(PendingToolUse {
                        id: block.id.unwrap_or_default(),
                        name: block.name.unwrap_or_default(),
                        input_json: String::new(),
                    });
                }
            }
            None
        }
        "content_block_delta" => {
            let event: ContentBlockDeltaEvent = match serde_json::from_str(data) {
                Ok(v) => v,
                Err(e) => return Some(Err(e.into())),
            };
            if let Some(delta) = event.delta {
                match delta.delta_type.as_deref() {
                    Some("input_json_delta") => {
                        if let Some(ref mut pending) = pending_tool_use {
                            if let Some(partial) = delta.partial_json {
                                pending.input_json.push_str(&partial);
                            }
                        }
                        None
                    }
                    _ => {
                        // Text delta (or unknown delta type)
                        delta.text.map(|text| Ok(StreamChunk::TextDelta(text)))
                    }
                }
            } else {
                None
            }
        }
        "content_block_stop" => {
            if let Some(pending) = pending_tool_use.take() {
                Some(Ok(StreamChunk::ToolUse {
                    id: pending.id,
                    name: pending.name,
                    input: pending.input_json,
                }))
            } else {
                None
            }
        }
        "message_delta" => {
            let event: MessageDeltaEvent = match serde_json::from_str(data) {
                Ok(v) => v,
                Err(e) => return Some(Err(e.into())),
            };
            if let Some(tokens) = event.usage.and_then(|u| u.output_tokens) {
                *output_tokens = Some(tokens);
            }
            if let Some(reason) = event
                .delta
                .and_then(|d| d.stop_reason)
                .and_then(|s| StopReason::from_api(&s))
            {
                *stop_reason = Some(reason);
            }
            None
        }
        "message_stop" => Some(Ok(StreamChunk::Done {
            output_tokens: *output_tokens,
            stop_reason: stop_reason.take(),
        })),
        "error" => {
            let event: ErrorEvent = match serde_json::from_str(data) {
                Ok(v) => v,
                Err(e) => return Some(Err(e.into())),
            };
            let msg = event
                .error
                .and_then(|e| e.message)
                .unwrap_or_else(|| "Unknown error".to_string());
            Some(Ok(StreamChunk::Error(msg)))
        }
        _ => None,
    }
}

#[cfg(test)]
#[path = "anthropic_tests.rs"]
mod tests;
