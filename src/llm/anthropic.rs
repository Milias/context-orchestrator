use crate::llm::{ChatConfig, ChatMessage, LlmProvider, StreamChunk};
use async_trait::async_trait;
use futures::stream::{Stream, StreamExt};
use reqwest::Client;
use serde_json::Value;
use std::pin::Pin;

pub struct AnthropicProvider {
    api_key: String,
    client: Client,
}

impl AnthropicProvider {
    pub fn new() -> anyhow::Result<Self> {
        let api_key = std::env::var("ANTHROPIC_API_KEY")
            .map_err(|_| anyhow::anyhow!("ANTHROPIC_API_KEY environment variable not set"))?;
        let client = Client::builder()
            .connect_timeout(std::time::Duration::from_secs(10))
            .build()?;
        Ok(Self { api_key, client })
    }
}

#[async_trait]
impl LlmProvider for AnthropicProvider {
    async fn chat(
        &self,
        messages: Vec<ChatMessage>,
        config: &ChatConfig,
    ) -> anyhow::Result<Pin<Box<dyn Stream<Item = anyhow::Result<StreamChunk>> + Send>>> {
        let mut body = serde_json::json!({
            "model": config.model,
            "max_tokens": config.max_tokens,
            "messages": messages,
            "stream": true,
        });
        if let Some(ref system) = config.system_prompt {
            body["system"] = serde_json::Value::String(system.clone());
        }

        let response = self
            .client
            .post("https://api.anthropic.com/v1/messages")
            .header("x-api-key", &self.api_key)
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

    fn name(&self) -> &str {
        "anthropic"
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

    let json: Value = match serde_json::from_str(data) {
        Ok(v) => v,
        Err(e) => return Some(Err(e.into())),
    };

    match event_type {
        "message_start" => {
            if let Some(tokens) = json["message"]["usage"]["input_tokens"].as_u64() {
                *input_tokens = Some(tokens as u32);
            }
            None
        }
        "content_block_delta" => {
            if let Some(text) = json["delta"]["text"].as_str() {
                Some(Ok(StreamChunk::TextDelta(text.to_string())))
            } else {
                None
            }
        }
        "message_delta" => {
            if let Some(tokens) = json["usage"]["output_tokens"].as_u64() {
                *output_tokens = Some(tokens as u32);
            }
            None
        }
        "message_stop" => Some(Ok(StreamChunk::Done {
            input_tokens: *input_tokens,
            output_tokens: *output_tokens,
        })),
        "error" => {
            let msg = json["error"]["message"]
                .as_str()
                .unwrap_or("Unknown error");
            Some(Ok(StreamChunk::Error(msg.to_string())))
        }
        _ => None, // message_start, content_block_start, content_block_stop, ping
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
        assert!(
            matches!(result, Some(Ok(StreamChunk::TextDelta(ref t))) if t == "Hello")
        );
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
        assert!(
            matches!(result, Some(Ok(StreamChunk::Done { input_tokens: Some(25), output_tokens: Some(100) })))
        );
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
        assert!(
            matches!(result, Some(Ok(StreamChunk::Error(ref e))) if e == "Overloaded")
        );
    }

    #[tokio::test]
    async fn test_real_api_call() {
        if std::env::var("ANTHROPIC_API_KEY").is_err() {
            eprintln!("Skipping: ANTHROPIC_API_KEY not set");
            return;
        }
        let provider = AnthropicProvider::new().unwrap();
        let messages = vec![ChatMessage {
            role: "user".to_string(),
            content: "Say hello in exactly 3 words.".to_string(),
        }];
        let config = ChatConfig::default();
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
