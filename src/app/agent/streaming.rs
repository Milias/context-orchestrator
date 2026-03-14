//! LLM streaming with retry, reconnection, and cancellation.
//!
//! All functions receive an `AgentContext` for sending events back to the main
//! loop. This avoids threading `(task_tx, agent_id)` through every function.

use crate::graph::StopReason;
use crate::llm::error::ApiError;
use crate::llm::retry::RetryConfig;
use crate::llm::{ChatConfig, ChatMessage, LlmProvider, StreamChunk};
use crate::tasks::{AgentEvent, AgentPhase, TaskMessage, ToolUseRecord};

use crate::app::think_splitter::ThinkSplitter;

use futures::StreamExt;
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

const MAX_STREAM_RETRIES: u32 = 2;

// ── AgentContext ─────────────────────────────────────────────────────

/// Context carried by an agent loop, used to send events back to the main loop.
/// Replaces threading `(task_tx, agent_id)` through every function.
pub(in crate::app) struct AgentContext {
    pub agent_id: Uuid,
    pub task_tx: mpsc::UnboundedSender<TaskMessage>,
}

impl AgentContext {
    /// Send an agent event to the main loop, tagged with this agent's ID.
    pub fn send(&self, event: AgentEvent) {
        let _ = self.task_tx.send(TaskMessage::Agent {
            agent_id: self.agent_id,
            event,
        });
    }
}

// ── StreamResult ────────────────────────────────────────────────────

/// Outcome of an LLM streaming call.
#[derive(Debug, PartialEq, Eq)]
pub(in crate::app) enum StreamOutcome {
    /// Normal completion with content.
    Success,
    /// User or system cancellation.
    Cancelled,
    /// API returned a non-retryable error. Agent should retry with rebuilt context.
    ApiError,
}

pub(in crate::app) struct StreamResult {
    pub response: String,
    pub think_text: String,
    pub input_tokens: Option<u32>,
    pub output_tokens: Option<u32>,
    pub tool_use_records: Vec<ToolUseRecord>,
    pub stop_reason: Option<StopReason>,
    pub outcome: StreamOutcome,
    /// Error message when `outcome` is `ApiError`. Used by the agent loop to
    /// record the error node synchronously before retrying.
    pub error_message: Option<String>,
    /// Phase ID for the Receiving phase, created during connection.
    /// Caller must send `PhaseCompleted` for this ID when streaming ends.
    pub recv_phase_id: Option<Uuid>,
}

// ── Public API ──────────────────────────────────────────────────────

pub(in crate::app) async fn stream_llm_response(
    provider: &Arc<dyn LlmProvider>,
    messages: Vec<ChatMessage>,
    config: &ChatConfig,
    ctx: &AgentContext,
    cancel_token: &CancellationToken,
) -> anyhow::Result<StreamResult> {
    match try_connect_chat(provider, &messages, config, ctx, cancel_token, "Connecting").await? {
        ConnectOutcome::Connected(stream, recv_phase_id) => {
            consume_stream(
                stream,
                recv_phase_id,
                provider,
                &messages,
                config,
                ctx,
                cancel_token,
            )
            .await
        }
        ConnectOutcome::ApiError(msg) => Ok(api_error_result(msg)),
        ConnectOutcome::Cancelled => Ok(cancelled_result()),
    }
}

// ── Stream consumption ──────────────────────────────────────────────

async fn consume_stream(
    mut stream: ChatStream,
    mut recv_phase_id: Uuid,
    provider: &Arc<dyn LlmProvider>,
    messages: &[ChatMessage],
    config: &ChatConfig,
    ctx: &AgentContext,
    cancel_token: &CancellationToken,
) -> anyhow::Result<StreamResult> {
    let mut state = StreamState::new();

    loop {
        match stream.next().await {
            Some(Ok(StreamChunk::TextDelta(text))) => {
                state.think_splitter.push(&text);
                if state.last_send.elapsed() >= state.send_budget {
                    send_delta(&state.think_splitter, ctx);
                    state.last_send = Instant::now();
                }
            }
            Some(Ok(StreamChunk::ToolUse { id, name, input })) => {
                state.tool_use_records.push(ToolUseRecord {
                    tool_call_id: Uuid::new_v4(),
                    api_id: id,
                    name,
                    input,
                });
            }
            Some(Ok(StreamChunk::Done {
                input_tokens: it,
                output_tokens: ot,
                stop_reason: sr,
            })) => {
                state.input_tokens = it;
                state.output_tokens = ot;
                state.stop_reason = sr;
                break;
            }
            Some(Ok(StreamChunk::Error(e))) => {
                if let Some(r) = try_reconnect(
                    &mut state.retries,
                    provider,
                    messages,
                    config,
                    ctx,
                    cancel_token,
                )
                .await?
                {
                    ctx.send(AgentEvent::PhaseCompleted {
                        phase_id: recv_phase_id,
                    });
                    (stream, recv_phase_id) = r;
                    state.think_splitter = ThinkSplitter::new();
                    continue;
                }
                state.error_message = Some(format!("Stream error: {e}"));
                break;
            }
            Some(Err(e)) => {
                let retryable = e
                    .downcast_ref::<ApiError>()
                    .is_some_and(ApiError::is_retryable);
                if retryable {
                    if let Some(r) = try_reconnect(
                        &mut state.retries,
                        provider,
                        messages,
                        config,
                        ctx,
                        cancel_token,
                    )
                    .await?
                    {
                        ctx.send(AgentEvent::PhaseCompleted {
                            phase_id: recv_phase_id,
                        });
                        (stream, recv_phase_id) = r;
                        state.think_splitter = ThinkSplitter::new();
                        continue;
                    }
                }
                state.error_message = Some(format_error(&e));
                break;
            }
            None => break,
        }
    }

    send_delta(&state.think_splitter, ctx);
    Ok(state.into_result(recv_phase_id))
}

struct StreamState {
    think_splitter: ThinkSplitter,
    input_tokens: Option<u32>,
    output_tokens: Option<u32>,
    tool_use_records: Vec<ToolUseRecord>,
    stop_reason: Option<StopReason>,
    retries: u32,
    last_send: Instant,
    send_budget: Duration,
    /// Set when the stream broke due to an error (reconnection exhausted).
    error_message: Option<String>,
}

impl StreamState {
    fn new() -> Self {
        Self {
            think_splitter: ThinkSplitter::new(),
            input_tokens: None,
            output_tokens: None,
            tool_use_records: Vec::new(),
            stop_reason: None,
            retries: 0,
            last_send: Instant::now(),
            send_budget: Duration::from_millis(16),
            error_message: None,
        }
    }

    fn into_result(self, recv_phase_id: Uuid) -> StreamResult {
        let (response, think_text) = self.think_splitter.finish();
        let (outcome, error_message) = if let Some(msg) = self.error_message {
            (StreamOutcome::ApiError, Some(msg))
        } else {
            (StreamOutcome::Success, None)
        };
        StreamResult {
            response,
            think_text,
            input_tokens: self.input_tokens,
            output_tokens: self.output_tokens,
            tool_use_records: self.tool_use_records,
            stop_reason: self.stop_reason,
            outcome,
            error_message,
            recv_phase_id: Some(recv_phase_id),
        }
    }
}

type ChatStream = Pin<Box<dyn futures::Stream<Item = anyhow::Result<StreamChunk>> + Send>>;

// ── Connection helpers ──────────────────────────────────────────────

/// Outcome of a connection attempt.
enum ConnectOutcome {
    Connected(ChatStream, Uuid),
    /// Non-retryable API error with the formatted message.
    ApiError(String),
    Cancelled,
}

/// Attempt a stream reconnection if retries remain. Returns `None` when exhausted or cancelled.
async fn try_reconnect(
    retries: &mut u32,
    provider: &Arc<dyn LlmProvider>,
    messages: &[ChatMessage],
    config: &ChatConfig,
    ctx: &AgentContext,
    cancel_token: &CancellationToken,
) -> anyhow::Result<Option<(ChatStream, Uuid)>> {
    if *retries >= MAX_STREAM_RETRIES {
        return Ok(None);
    }
    *retries += 1;
    match try_connect_chat(
        provider,
        messages,
        config,
        ctx,
        cancel_token,
        "Reconnecting",
    )
    .await?
    {
        ConnectOutcome::Connected(stream, phase_id) => Ok(Some((stream, phase_id))),
        ConnectOutcome::ApiError(_) | ConnectOutcome::Cancelled => Ok(None),
    }
}

/// Try to establish a chat stream with retry and cancellation.
/// On success returns `Connected(stream, recv_phase_id)`.
/// On non-retryable API error returns `ApiError` (after sending `AgentEvent::ApiError`).
/// On cancellation returns `Cancelled`.
async fn try_connect_chat(
    provider: &Arc<dyn LlmProvider>,
    messages: &[ChatMessage],
    config: &ChatConfig,
    ctx: &AgentContext,
    cancel_token: &CancellationToken,
    context_label: &str,
) -> anyhow::Result<ConnectOutcome> {
    let retry_config = RetryConfig::default();

    for attempt in 1..=retry_config.max_attempts {
        let connect_phase = Uuid::new_v4();
        ctx.send(AgentEvent::Progress {
            phase_id: connect_phase,
            phase: AgentPhase::Connecting {
                attempt,
                max: retry_config.max_attempts,
            },
        });

        match provider.chat(messages.to_vec(), config).await {
            Ok(s) => {
                ctx.send(AgentEvent::PhaseCompleted {
                    phase_id: connect_phase,
                });
                let recv_phase = Uuid::new_v4();
                ctx.send(AgentEvent::Progress {
                    phase_id: recv_phase,
                    phase: AgentPhase::Receiving,
                });
                return Ok(ConnectOutcome::Connected(s, recv_phase));
            }
            Err(e) => {
                let retryable = e
                    .downcast_ref::<ApiError>()
                    .is_some_and(ApiError::is_retryable);

                if !retryable || attempt == retry_config.max_attempts {
                    let msg = format_error(&e);
                    ctx.send(AgentEvent::ApiError {
                        phase_id: connect_phase,
                        message: msg.clone(),
                    });
                    return Ok(ConnectOutcome::ApiError(msg));
                }
                ctx.send(AgentEvent::PhaseCompleted {
                    phase_id: connect_phase,
                });

                let delay = retry_config.delay_for(attempt - 1, e.downcast_ref::<ApiError>());
                ctx.send(AgentEvent::StatusMessage(format!(
                    "{context_label} ({attempt}/{})... {}",
                    retry_config.max_attempts,
                    format_error(&e)
                )));

                let cancelled = tokio::select! {
                    () = tokio::time::sleep(delay) => false,
                    () = cancel_token.cancelled() => true,
                };
                if cancelled {
                    return Ok(ConnectOutcome::Cancelled);
                }
            }
        }
    }

    Ok(ConnectOutcome::Cancelled)
}

// ── Helpers ─────────────────────────────────────────────────────────

fn cancelled_result() -> StreamResult {
    StreamResult {
        response: String::new(),
        think_text: String::new(),
        input_tokens: None,
        output_tokens: None,
        tool_use_records: Vec::new(),
        stop_reason: None,
        outcome: StreamOutcome::Cancelled,
        error_message: None,
        recv_phase_id: None,
    }
}

fn api_error_result(message: String) -> StreamResult {
    StreamResult {
        response: String::new(),
        think_text: String::new(),
        input_tokens: None,
        output_tokens: None,
        tool_use_records: Vec::new(),
        stop_reason: None,
        outcome: StreamOutcome::ApiError,
        error_message: Some(message),
        recv_phase_id: None,
    }
}

fn send_delta(splitter: &ThinkSplitter, ctx: &AgentContext) {
    ctx.send(AgentEvent::StreamDelta {
        text: splitter.visible().to_string(),
        is_thinking: splitter.is_thinking(),
    });
}

pub(in crate::app) fn format_error(e: &anyhow::Error) -> String {
    e.downcast_ref::<ApiError>()
        .map_or_else(|| format!("{e}"), ToString::to_string)
}
