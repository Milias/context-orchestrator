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

pub(in crate::app) struct StreamResult {
    pub response: String,
    pub think_text: String,
    pub output_tokens: Option<u32>,
    pub tool_use_records: Vec<ToolUseRecord>,
    pub stop_reason: Option<StopReason>,
    pub cancelled: bool,
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
    let Some((stream, recv_phase_id)) =
        try_connect_chat(provider, &messages, config, ctx, cancel_token, "Connecting").await?
    else {
        return Ok(cancelled_result());
    };

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
                output_tokens: ot,
                stop_reason: sr,
            })) => {
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
                if state.retries > MAX_STREAM_RETRIES {
                    ctx.send(AgentEvent::Error(format!("Stream error: {e}")));
                }
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
                ctx.send(AgentEvent::Error(format_error(&e)));
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
    output_tokens: Option<u32>,
    tool_use_records: Vec<ToolUseRecord>,
    stop_reason: Option<StopReason>,
    retries: u32,
    last_send: Instant,
    send_budget: Duration,
}

impl StreamState {
    fn new() -> Self {
        Self {
            think_splitter: ThinkSplitter::new(),
            output_tokens: None,
            tool_use_records: Vec::new(),
            stop_reason: None,
            retries: 0,
            last_send: Instant::now(),
            send_budget: Duration::from_millis(16),
        }
    }

    fn into_result(self, recv_phase_id: Uuid) -> StreamResult {
        let (response, think_text) = self.think_splitter.finish();
        StreamResult {
            response,
            think_text,
            output_tokens: self.output_tokens,
            tool_use_records: self.tool_use_records,
            stop_reason: self.stop_reason,
            cancelled: false,
            recv_phase_id: Some(recv_phase_id),
        }
    }
}

type ChatStream = Pin<Box<dyn futures::Stream<Item = anyhow::Result<StreamChunk>> + Send>>;

// ── Connection helpers ──────────────────────────────────────────────

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
    try_connect_chat(
        provider,
        messages,
        config,
        ctx,
        cancel_token,
        "Reconnecting",
    )
    .await
}

/// Try to establish a chat stream with retry and cancellation.
/// On success returns `(stream, recv_phase_id)` — the Receiving phase is left Running.
async fn try_connect_chat(
    provider: &Arc<dyn LlmProvider>,
    messages: &[ChatMessage],
    config: &ChatConfig,
    ctx: &AgentContext,
    cancel_token: &CancellationToken,
    context_label: &str,
) -> anyhow::Result<Option<(ChatStream, Uuid)>> {
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
                return Ok(Some((s, recv_phase)));
            }
            Err(e) => {
                let retryable = e
                    .downcast_ref::<ApiError>()
                    .is_some_and(ApiError::is_retryable);

                if !retryable || attempt == retry_config.max_attempts {
                    ctx.send(AgentEvent::PhaseCompleted {
                        phase_id: connect_phase,
                    });
                    ctx.send(AgentEvent::Error(format_error(&e)));
                    return Ok(None);
                }
                ctx.send(AgentEvent::PhaseCompleted {
                    phase_id: connect_phase,
                });

                let delay = retry_config.delay_for(attempt - 1, e.downcast_ref::<ApiError>());
                ctx.send(AgentEvent::Error(format!(
                    "{context_label} ({attempt}/{})... {}",
                    retry_config.max_attempts,
                    format_error(&e)
                )));

                let cancelled = tokio::select! {
                    () = tokio::time::sleep(delay) => false,
                    () = cancel_token.cancelled() => true,
                };
                if cancelled {
                    return Ok(None);
                }
            }
        }
    }

    Ok(None)
}

// ── Helpers ─────────────────────────────────────────────────────────

fn cancelled_result() -> StreamResult {
    StreamResult {
        response: String::new(),
        think_text: String::new(),
        output_tokens: None,
        tool_use_records: Vec::new(),
        stop_reason: None,
        cancelled: true,
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
