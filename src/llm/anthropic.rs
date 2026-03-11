use crate::config::AppConfig;
use crate::llm::{ChatConfig, ChatMessage, LlmProvider, StreamChunk};
use async_trait::async_trait;
use futures::stream::{Stream, StreamExt};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::pin::Pin;

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
            .connect_timeout(std::time::Duration::from_secs(10))
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
}

#[derive(Serialize)]
struct CountTokensRequest {
    model: String,
    messages: Vec<ChatMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    system: Option<String>,
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
        let body = MessagesRequest {
            model: config.model.clone(),
            max_tokens: config.max_tokens,
            messages,
            stream: true,
            system: config.system_prompt.clone(),
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
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("Anthropic API error {}: {}", status, body);
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
    ) -> anyhow::Result<u32> {
        let body = CountTokensRequest {
            model: model.to_string(),
            messages: messages.to_vec(),
            system: system_prompt.map(|s| s.to_string()),
        };

        let response = self
            .client
            .post(format!("{}/v1/messages/count_tokens", self.base_url))
            .header("x-api-key", &self.api_key)
            .header("authorization", format!("Bearer {}", self.api_key))
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("Token count API error {}: {}", status, body);
        }

        let result: CountTokensResponse = response.json().await?;
        Ok(result.input_tokens)
    }
}

struct SseState<S> {
    stream: Pin<Box<S>>,
    buffer: String,
    input_tokens: Option<u32>,
    output_tokens: Option<u32>,
}

fn sse_to_stream_chunks(
    byte_stream: impl Stream<Item = Result<bytes::Bytes, reqwest::Error>> + Send + 'static,
) -> impl Stream<Item = anyhow::Result<StreamChunk>> + Send + 'static {
    futures::stream::unfold(
        SseState {
            stream: Box::pin(byte_stream),
            buffer: String::new(),
            input_tokens: None,
            output_tokens: None,
        },
        |mut state| async move {
            loop {
                // Check if buffer contains a complete event
                if let Some(pos) = state.buffer.find("\n\n") {
                    let event_text = state.buffer[..pos].to_string();
                    state.buffer = state.buffer[pos + 2..].to_string();

                    if let Some(chunk) = parse_sse_event(
                        &event_text,
                        &mut state.input_tokens,
                        &mut state.output_tokens,
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
                        return Some((Err(anyhow::Error::from(e)), state));
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
struct MessageStartEvent {
    message: Option<MessagePayload>,
}

#[derive(Deserialize)]
struct MessagePayload {
    usage: Option<UsagePayload>,
}

#[derive(Deserialize)]
struct ContentBlockDeltaEvent {
    delta: Option<DeltaPayload>,
}

#[derive(Deserialize)]
struct DeltaPayload {
    text: Option<String>,
}

#[derive(Deserialize)]
struct MessageDeltaEvent {
    usage: Option<UsagePayload>,
}

#[derive(Deserialize)]
struct UsagePayload {
    input_tokens: Option<u32>,
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
    input_tokens: &mut Option<u32>,
    output_tokens: &mut Option<u32>,
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
        "message_start" => {
            let event: MessageStartEvent = match serde_json::from_str(data) {
                Ok(v) => v,
                Err(e) => return Some(Err(e.into())),
            };
            if let Some(tokens) = event.message.and_then(|m| m.usage).and_then(|u| u.input_tokens)
            {
                *input_tokens = Some(tokens);
            }
            None
        }
        "content_block_delta" => {
            let event: ContentBlockDeltaEvent = match serde_json::from_str(data) {
                Ok(v) => v,
                Err(e) => return Some(Err(e.into())),
            };
            event
                .delta
                .and_then(|d| d.text)
                .map(|text| Ok(StreamChunk::TextDelta(text)))
        }
        "message_delta" => {
            let event: MessageDeltaEvent = match serde_json::from_str(data) {
                Ok(v) => v,
                Err(e) => return Some(Err(e.into())),
            };
            if let Some(tokens) = event.usage.and_then(|u| u.output_tokens) {
                *output_tokens = Some(tokens);
            }
            None
        }
        "message_stop" => Some(Ok(StreamChunk::Done {
            input_tokens: *input_tokens,
            output_tokens: *output_tokens,
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
mod tests {
    use super::*;

    #[test]
    fn test_parse_text_delta() {
        let event = "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"Hello\"}}";
        let mut it = None;
        let mut ot = None;
        let result = parse_sse_event(event, &mut it, &mut ot);
        assert!(matches!(result, Some(Ok(StreamChunk::TextDelta(ref t))) if t == "Hello"));
    }

    #[test]
    fn test_parse_message_start_captures_input_tokens() {
        let event = "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":25}}}";
        let mut it = None;
        let mut ot = None;
        let result = parse_sse_event(event, &mut it, &mut ot);
        assert!(result.is_none());
        assert_eq!(it, Some(25));
    }

    #[test]
    fn test_parse_message_delta_captures_output_tokens() {
        let event = "event: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":100}}";
        let mut it = None;
        let mut ot = None;
        let result = parse_sse_event(event, &mut it, &mut ot);
        assert!(result.is_none());
        assert_eq!(ot, Some(100));
    }

    #[test]
    fn test_parse_message_stop() {
        let event = "event: message_stop\ndata: {\"type\":\"message_stop\"}";
        let mut it = Some(25);
        let mut ot = Some(100);
        let result = parse_sse_event(event, &mut it, &mut ot);
        assert!(matches!(
            result,
            Some(Ok(StreamChunk::Done {
                input_tokens: Some(25),
                output_tokens: Some(100)
            }))
        ));
    }

    #[test]
    fn test_parse_ping_ignored() {
        let event = "event: ping\ndata: {\"type\":\"ping\"}";
        let mut it = None;
        let mut ot = None;
        let result = parse_sse_event(event, &mut it, &mut ot);
        assert!(result.is_none());
    }

    #[test]
    fn test_parse_error_event() {
        let event = "event: error\ndata: {\"type\":\"error\",\"error\":{\"type\":\"overloaded_error\",\"message\":\"Overloaded\"}}";
        let mut it = None;
        let mut ot = None;
        let result = parse_sse_event(event, &mut it, &mut ot);
        assert!(matches!(result, Some(Ok(StreamChunk::Error(ref e))) if e == "Overloaded"));
    }

    #[tokio::test]
    async fn test_real_api_call() {
        if std::env::var("ANTHROPIC_AUTH_TOKEN").is_err()
            && std::env::var("ANTHROPIC_API_KEY").is_err()
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
                StreamChunk::Error(e) => panic!("Error: {}", e),
            }
        }
        assert!(!full_text.is_empty());
        eprintln!("Response: {}", full_text);
    }
}
