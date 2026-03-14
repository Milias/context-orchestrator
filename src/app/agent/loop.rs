//! Background agent loop: context building → LLM streaming → tool dispatch → repeat.
//!
//! Each agent loop runs as an independent tokio task. It communicates with the
//! main loop via [`AgentContext`] (events out) and a tool-result receiver (results in).

use crate::graph::tool::result::ToolResultContent;
use crate::graph::tool::types::ToolCallStatus;
use crate::graph::{parse_tool_arguments, EdgeKind, Node, Role, StopReason};
use crate::llm::{ChatConfig, ChatMessage, LlmProvider, ToolDefinition};
use crate::tasks::{AgentEvent, AgentPhase, AgentToolResult};

use super::streaming::{self as agent_streaming, AgentContext, StreamResult};
use crate::app::context;
use crate::app::SharedGraph;

use chrono::Utc;
use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

/// Full configuration for spawning an agent loop.
pub(in crate::app) struct AgentLoopConfig {
    pub graph: SharedGraph,
    pub provider: Arc<dyn LlmProvider>,
    pub model: String,
    pub max_tokens: u32,
    pub max_context_tokens: u32,
    pub max_tool_loop_iterations: usize,
    pub tools: Vec<ToolDefinition>,
    /// The anchor node ID (user message).
    pub anchor_id: Uuid,
    pub agent_id: Uuid,
}

/// Spawn the agent loop as a background task.
pub(in crate::app) fn spawn_agent_loop(
    config: AgentLoopConfig,
    task_tx: mpsc::UnboundedSender<TaskMessage>,
    tool_result_rx: mpsc::UnboundedReceiver<AgentToolResult>,
    cancel_token: CancellationToken,
) {
    let agent_id = config.agent_id;
    let ctx = AgentContext { agent_id, task_tx };
    let graph = Arc::clone(&config.graph);
    let provider = Arc::clone(&config.provider);
    let anchor_id = config.anchor_id;
    tokio::spawn(async move {
        let result = run_agent_loop(
            &graph,
            provider,
            config,
            anchor_id,
            &ctx,
            tool_result_rx,
            cancel_token,
        )
        .await;
        if let Err(e) = &result {
            ctx.send(AgentEvent::Error(e.to_string()));
        }
        ctx.send(AgentEvent::Finished);
    });
}

/// Max times we auto-continue when the LLM hits `max_tokens`.
const MAX_CONTINUATIONS: u32 = 3;

use crate::tasks::TaskMessage;

async fn run_agent_loop(
    graph: &SharedGraph,
    provider: Arc<dyn LlmProvider>,
    config: AgentLoopConfig,
    user_msg_id: Uuid,
    ctx: &AgentContext,
    mut tool_result_rx: mpsc::UnboundedReceiver<AgentToolResult>,
    cancel_token: CancellationToken,
) -> anyhow::Result<()> {
    // Fire-and-forget: token counting is independent of context building.
    spawn_count_user_tokens(
        Arc::clone(graph),
        Arc::clone(&provider),
        config.model.clone(),
        user_msg_id,
        ctx,
    );

    let chat_config = ChatConfig {
        model: config.model.clone(),
        max_tokens: config.max_tokens,
        system_prompt: None,
        tools: config.tools.clone(),
    };

    let mut continuation_count: u32 = 0;

    for _ in 0..config.max_tool_loop_iterations {
        let ctx_phase = Uuid::new_v4();
        ctx.send(AgentEvent::Progress {
            phase_id: ctx_phase,
            phase: AgentPhase::BuildingContext,
        });

        let (system_prompt, messages) = {
            let g = graph.read();
            context::extract_messages(&g, Some(config.agent_id))
        };
        let (system_prompt, messages) = context::finalize_context(
            system_prompt,
            messages,
            provider.as_ref(),
            &config.model,
            config.max_context_tokens,
            &config.tools,
        )
        .await?;

        ctx.send(AgentEvent::PhaseCompleted {
            phase_id: ctx_phase,
        });

        let loop_config = ChatConfig {
            system_prompt,
            ..chat_config.clone()
        };

        let result = agent_streaming::stream_llm_response(
            &provider,
            messages,
            &loop_config,
            ctx,
            &cancel_token,
        )
        .await?;

        if let Some(recv_id) = result.recv_phase_id {
            ctx.send(AgentEvent::PhaseCompleted { phase_id: recv_id });
        }

        if result.cancelled || (result.response.is_empty() && result.tool_use_records.is_empty()) {
            break;
        }

        let assistant_id = apply_iteration_to_graph(graph, &result, &loop_config, ctx)?;

        let is_tool_use =
            result.stop_reason == Some(StopReason::ToolUse) && !result.tool_use_records.is_empty();
        let is_truncated = result.stop_reason == Some(StopReason::MaxTokens);

        if is_truncated {
            continuation_count += 1;
            if continuation_count > MAX_CONTINUATIONS {
                ctx.send(AgentEvent::Error(
                    "Max continuations reached after repeated truncation".into(),
                ));
                break;
            }
        } else {
            continuation_count = 0;
        }

        if is_tool_use {
            // Execute tool calls and wait for results before next iteration.
        } else if is_truncated {
            continue;
        } else {
            break;
        }

        let timed_out =
            dispatch_and_wait_for_tools(graph, &result, assistant_id, &mut tool_result_rx, ctx)
                .await;

        if timed_out {
            break;
        }
    }

    Ok(())
}

/// Spawn token counting as a fire-and-forget task.
fn spawn_count_user_tokens(
    graph: SharedGraph,
    provider: Arc<dyn LlmProvider>,
    model: String,
    user_msg_id: Uuid,
    ctx: &AgentContext,
) {
    let ctx_tx = ctx.task_tx.clone();
    let ctx_id = ctx.agent_id;
    tokio::spawn(async move {
        let fire_ctx = AgentContext {
            agent_id: ctx_id,
            task_tx: ctx_tx,
        };
        let phase_id = Uuid::new_v4();
        fire_ctx.send(AgentEvent::Progress {
            phase_id,
            phase: AgentPhase::CountingTokens,
        });
        let content = {
            let g = graph.read();
            if let Some(Node::Message { content, .. }) = g.node(user_msg_id) {
                Some(content.clone())
            } else {
                None
            }
        };
        if let Some(content) = content {
            let msg = vec![ChatMessage::text(Role::User, &content)];
            if let Ok(count) = provider.count_tokens(&msg, &model, None, &[]).await {
                fire_ctx.send(AgentEvent::UserTokensCounted {
                    node_id: user_msg_id,
                    count,
                });
            }
        }
        fire_ctx.send(AgentEvent::PhaseCompleted { phase_id });
    });
}

/// Add the assistant response and think block to the shared graph.
fn apply_iteration_to_graph(
    graph: &SharedGraph,
    result: &StreamResult,
    config: &ChatConfig,
    ctx: &AgentContext,
) -> anyhow::Result<Uuid> {
    let assistant_id = Uuid::new_v4();

    {
        let mut g = graph.write();
        let leaf = g.active_leaf()?;
        let assistant_node = Node::Message {
            id: assistant_id,
            role: Role::Assistant,
            content: result.response.clone(),
            created_at: Utc::now(),
            model: Some(config.model.clone()),
            input_tokens: None,
            output_tokens: result.output_tokens,
            stop_reason: result.stop_reason,
        };
        g.add_message(leaf, assistant_node)?;

        if !result.think_text.is_empty() {
            let think_node = Node::ThinkBlock {
                id: Uuid::new_v4(),
                content: result.think_text.clone(),
                parent_message_id: assistant_id,
                created_at: Utc::now(),
            };
            let think_id = g.add_node(think_node);
            g.add_edge(think_id, assistant_id, EdgeKind::ThinkingOf)?;
        }
    }

    ctx.send(AgentEvent::IterationCommitted {
        assistant_id,
        stop_reason: result.stop_reason,
    });

    Ok(assistant_id)
}

/// Add tool call nodes to the graph, send dispatch notifications, wait for results.
async fn dispatch_and_wait_for_tools(
    graph: &SharedGraph,
    result: &StreamResult,
    assistant_id: Uuid,
    tool_result_rx: &mut mpsc::UnboundedReceiver<AgentToolResult>,
    ctx: &AgentContext,
) -> bool {
    let mut pending_ids = HashSet::new();
    {
        let mut g = graph.write();
        for record in &result.tool_use_records {
            let args = parse_tool_arguments(&record.name, &record.input);
            g.add_tool_call(
                record.tool_call_id,
                assistant_id,
                args.clone(),
                Some(record.api_id.clone()),
            );
            pending_ids.insert(record.tool_call_id);

            ctx.send(AgentEvent::ToolCallDispatched {
                tool_call_id: record.tool_call_id,
                arguments: args,
            });
        }
    }

    let tools_phase = Uuid::new_v4();
    ctx.send(AgentEvent::Progress {
        phase_id: tools_phase,
        phase: AgentPhase::ExecutingTools {
            count: pending_ids.len(),
        },
    });

    let timed_out = wait_for_tool_results(&mut pending_ids, tool_result_rx, ctx, graph).await;
    ctx.send(AgentEvent::PhaseCompleted {
        phase_id: tools_phase,
    });
    timed_out
}

/// Wait for all pending tool calls to complete. Returns `true` if timed out.
async fn wait_for_tool_results(
    pending_ids: &mut HashSet<Uuid>,
    tool_result_rx: &mut mpsc::UnboundedReceiver<AgentToolResult>,
    ctx: &AgentContext,
    graph: &SharedGraph,
) -> bool {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(60);

    while !pending_ids.is_empty() {
        tokio::select! {
            Some(result) = tool_result_rx.recv() => {
                pending_ids.remove(&result.tool_call_id);
            }
            () = tokio::time::sleep_until(deadline) => {
                timeout_pending_tools(graph, pending_ids);
                ctx.send(AgentEvent::Error("Tool call(s) timed out".into()));
                return true;
            }
        }
    }

    false
}

/// Write timeout results to the shared graph for tools that didn't complete in time.
fn timeout_pending_tools(graph: &SharedGraph, pending_ids: &mut HashSet<Uuid>) {
    let mut g = graph.write();
    for tc_id in pending_ids.drain() {
        if let Some(Node::ToolCall { status, .. }) = g.node(tc_id) {
            if *status == ToolCallStatus::Completed || *status == ToolCallStatus::Failed {
                continue;
            }
        }
        let _ = g.update_tool_call_status(tc_id, ToolCallStatus::Failed, Some(Utc::now()));
        g.add_tool_result(
            tc_id,
            ToolResultContent::text("Tool execution timed out"),
            true,
        );
    }
}
