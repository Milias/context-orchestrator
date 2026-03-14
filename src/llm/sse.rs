//! SSE (Server-Sent Events) parser for the Anthropic streaming API.
//!
//! Converts raw byte streams into typed `StreamChunk` values. Serde structs
//! keep `Option<String>` fields for forward compatibility with new API types.
//! Typed enums (`SseEventType`, `ContentBlockType`, `DeltaType`) convert at
//! the processing boundary — same pattern as `StopReason::from_api`.

use crate::graph::StopReason;
use crate::llm::error::ApiError;
use crate::llm::StreamChunk;
use futures::stream::{Stream, StreamExt};
use serde::Deserialize;
use std::pin::Pin;

// ── Internal state ──────────────────────────────────────────────────

struct SseState<S> {
    stream: Pin<Box<S>>,
    buffer: String,
    input_tokens: Option<u32>,
    output_tokens: Option<u32>,
    stop_reason: Option<StopReason>,
    /// Pending `tool_use` block being accumulated across SSE events.
    pending_tool_use: Option<PendingToolUse>,
}

pub(super) struct PendingToolUse {
    id: String,
    name: String,
    input_json: String,
}

// ── Public stream converter ─────────────────────────────────────────

/// Transform a raw byte stream into a stream of typed `StreamChunk` values.
pub(crate) fn sse_to_stream_chunks(
    byte_stream: impl Stream<Item = Result<bytes::Bytes, reqwest::Error>> + Send + 'static,
) -> impl Stream<Item = anyhow::Result<StreamChunk>> + Send + 'static {
    futures::stream::unfold(
        SseState {
            stream: Box::pin(byte_stream),
            buffer: String::new(),
            input_tokens: None,
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
                        &mut state.input_tokens,
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

// ── SSE type enums ──────────────────────────────────────────────────
// Serde structs keep `Option<String>` for forward compatibility with new API
// types. These enums convert at the processing boundary — same pattern as
// `StopReason::from_api`. Unknown variants are explicit, not silent fallthrough.

/// SSE event types from the Anthropic streaming API.
enum SseEventType {
    MessageStart,
    ContentBlockStart,
    ContentBlockDelta,
    ContentBlockStop,
    MessageDelta,
    MessageStop,
    Error,
    Unknown,
}

impl SseEventType {
    fn from_api(s: &str) -> Self {
        match s {
            "message_start" => Self::MessageStart,
            "content_block_start" => Self::ContentBlockStart,
            "content_block_delta" => Self::ContentBlockDelta,
            "content_block_stop" => Self::ContentBlockStop,
            "message_delta" => Self::MessageDelta,
            "message_stop" => Self::MessageStop,
            "error" => Self::Error,
            _ => Self::Unknown,
        }
    }
}

/// Content block types within `content_block_start` events.
/// `#[serde(other)]` catches new API types without deserialization failures.
#[derive(Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum ContentBlockType {
    Text,
    ToolUse,
    Thinking,
    #[serde(other)]
    Unknown,
}

/// Delta types within `content_block_delta` events.
/// `#[serde(other)]` catches new API types without deserialization failures.
#[derive(Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum DeltaType {
    TextDelta,
    InputJsonDelta,
    ThinkingDelta,
    #[serde(other)]
    Unknown,
}

// ── Serde deserialization structs ───────────────────────────────────
// Forward-compatible: unknown fields/types deserialize as `None`, not errors.

#[derive(Deserialize)]
struct ContentBlockStartEvent {
    content_block: Option<ContentBlockInfo>,
}

#[derive(Deserialize)]
struct ContentBlockInfo {
    #[serde(rename = "type")]
    block_type: Option<ContentBlockType>,
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
    delta_type: Option<DeltaType>,
    text: Option<String>,
    partial_json: Option<String>,
}

#[derive(Deserialize)]
struct MessageStartEvent {
    message: Option<MessageStartPayload>,
}

#[derive(Deserialize)]
struct MessageStartPayload {
    usage: Option<MessageStartUsage>,
}

/// Usage from `message_start` carries `input_tokens` (the full context window cost).
#[derive(Deserialize)]
struct MessageStartUsage {
    input_tokens: Option<u32>,
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

// ── Event parser ────────────────────────────────────────────────────

/// Parse a `content_block_delta` event. Accumulates tool input JSON or yields text.
fn handle_content_block_delta(
    data: &str,
    pending_tool_use: &mut Option<PendingToolUse>,
) -> Option<anyhow::Result<StreamChunk>> {
    let event: ContentBlockDeltaEvent = match serde_json::from_str(data) {
        Ok(v) => v,
        Err(e) => return Some(Err(e.into())),
    };
    let delta = event.delta?;
    match delta.delta_type {
        Some(DeltaType::InputJsonDelta) => {
            if let Some(ref mut pending) = pending_tool_use {
                if let Some(partial) = delta.partial_json {
                    pending.input_json.push_str(&partial);
                }
            }
            None
        }
        Some(DeltaType::TextDelta | DeltaType::Unknown) | None => {
            delta.text.map(|text| Ok(StreamChunk::TextDelta(text)))
        }
        Some(DeltaType::ThinkingDelta) => None,
    }
}

/// Parse a `message_delta` event. Captures output tokens and stop reason.
fn handle_message_delta(
    data: &str,
    output_tokens: &mut Option<u32>,
    stop_reason: &mut Option<StopReason>,
) -> Option<anyhow::Result<StreamChunk>> {
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

/// Parse a single SSE event text into a `StreamChunk`, if applicable.
/// Mutates accumulated state for multi-event constructs (tool use, message delta).
pub(super) fn parse_sse_event(
    event_text: &str,
    input_tokens: &mut Option<u32>,
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

    match SseEventType::from_api(event_type) {
        SseEventType::MessageStart => {
            let event: MessageStartEvent = match serde_json::from_str(data) {
                Ok(v) => v,
                Err(e) => return Some(Err(e.into())),
            };
            if let Some(tokens) = event
                .message
                .and_then(|m| m.usage)
                .and_then(|u| u.input_tokens)
            {
                *input_tokens = Some(tokens);
            }
            None
        }
        SseEventType::ContentBlockStart => {
            let event: ContentBlockStartEvent = match serde_json::from_str(data) {
                Ok(v) => v,
                Err(e) => return Some(Err(e.into())),
            };
            if let Some(block) = event.content_block {
                if block.block_type.as_ref() == Some(&ContentBlockType::ToolUse) {
                    *pending_tool_use = Some(PendingToolUse {
                        id: block.id.unwrap_or_default(),
                        name: block.name.unwrap_or_default(),
                        input_json: String::new(),
                    });
                }
            }
            None
        }
        SseEventType::ContentBlockDelta => handle_content_block_delta(data, pending_tool_use),
        SseEventType::ContentBlockStop => pending_tool_use.take().map(|pending| {
            Ok(StreamChunk::ToolUse {
                id: pending.id,
                name: pending.name,
                input: pending.input_json,
            })
        }),
        SseEventType::MessageDelta => handle_message_delta(data, output_tokens, stop_reason),
        SseEventType::MessageStop => Some(Ok(StreamChunk::Done {
            input_tokens: *input_tokens,
            output_tokens: *output_tokens,
            stop_reason: stop_reason.take(),
        })),
        SseEventType::Error => {
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
        SseEventType::Unknown => None,
    }
}
