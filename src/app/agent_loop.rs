use crate::graph::tool_types::{ToolCallStatus, ToolResultContent};
use crate::graph::{parse_tool_arguments, EdgeKind, Node, Role, StopReason};
use crate::llm::{ChatConfig, ChatMessage, LlmProvider, ToolDefinition};
use crate::tasks::{AgentEvent, AgentPhase, AgentToolResult, TaskMessage};

use super::agent_streaming::{self, send, StreamResult};
use super::context;
use super::SharedGraph;

use chrono::Utc;
use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

/// Configuration extracted from `AppConfig` for the agent loop.
pub(super) struct AgentLoopConfig {
    pub model: String,
    pub max_tokens: u32,
    pub max_context_tokens: u32,
    pub max_tool_loop_iterations: usize,
    pub tools: Vec<ToolDefinition>,
}

/// Spawn the agent loop as a background task.
pub(super) fn spawn_agent_loop(
    graph: SharedGraph,
    provider: Arc<dyn LlmProvider>,
    config: AgentLoopConfig,
    user_msg_id: Uuid,
    task_tx: mpsc::UnboundedSender<TaskMessage>,
    tool_result_rx: mpsc::UnboundedReceiver<AgentToolResult>,
    cancel_token: CancellationToken,
) {
    tokio::spawn(async move {
        let result = run_agent_loop(
            &graph,
            provider,
            config,
            user_msg_id,
            &task_tx,
            tool_result_rx,
            cancel_token,
        )
        .await;
        if let Err(e) = result {
            let _ = task_tx.send(TaskMessage::Agent(AgentEvent::Error(e.to_string())));
        }
        let _ = task_tx.send(TaskMessage::Agent(AgentEvent::Finished));
    });
}

/// Max times we auto-continue when the LLM hits `max_tokens`.
const MAX_CONTINUATIONS: u32 = 3;

async fn run_agent_loop(
    graph: &SharedGraph,
    provider: Arc<dyn LlmProvider>,
    config: AgentLoopConfig,
    user_msg_id: Uuid,
    task_tx: &mpsc::UnboundedSender<TaskMessage>,
    mut tool_result_rx: mpsc::UnboundedReceiver<AgentToolResult>,
    cancel_token: CancellationToken,
) -> anyhow::Result<()> {
    // Fire-and-forget: token counting is independent of context building.
    spawn_count_user_tokens(
        Arc::clone(graph),
        Arc::clone(&provider),
        config.model.clone(),
        user_msg_id,
        task_tx.clone(),
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
        send(
            task_tx,
            AgentEvent::Progress {
                phase_id: ctx_phase,
                phase: AgentPhase::BuildingContext,
            },
        );

        // Read graph under lock, then release before async token counting.
        let (system_prompt, messages) = {
            let g = graph.read();
            context::extract_messages(&g, &config.tools)
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

        send(
            task_tx,
            AgentEvent::PhaseCompleted {
                phase_id: ctx_phase,
            },
        );

        let loop_config = ChatConfig {
            system_prompt,
            ..chat_config.clone()
        };

        let result = agent_streaming::stream_llm_response(
            &provider,
            messages,
            &loop_config,
            task_tx,
            &cancel_token,
        )
        .await?;

        if let Some(recv_id) = result.recv_phase_id {
            send(task_tx, AgentEvent::PhaseCompleted { phase_id: recv_id });
        }

        if result.cancelled || (result.response.is_empty() && result.tool_use_records.is_empty()) {
            break;
        }

        let assistant_id = apply_iteration_to_graph(graph, &result, &loop_config, task_tx)?;

        let is_tool_use =
            result.stop_reason == Some(StopReason::ToolUse) && !result.tool_use_records.is_empty();
        let is_truncated = result.stop_reason == Some(StopReason::MaxTokens);

        if is_truncated {
            continuation_count += 1;
            if continuation_count > MAX_CONTINUATIONS {
                send(
                    task_tx,
                    AgentEvent::Error("Max continuations reached after repeated truncation".into()),
                );
                break;
            }
        } else {
            continuation_count = 0;
        }

        if is_tool_use {
            // Execute tool calls and wait for results before next iteration.
        } else if is_truncated {
            // Auto-continue: skip tool dispatch, loop directly to next iteration.
            continue;
        } else {
            break;
        }

        let timed_out =
            dispatch_and_wait_for_tools(graph, &result, assistant_id, &mut tool_result_rx, task_tx)
                .await;

        if timed_out {
            break;
        }
    }

    Ok(())
}

/// Spawn token counting as a fire-and-forget task. Runs concurrently with context building.
fn spawn_count_user_tokens(
    graph: SharedGraph,
    provider: Arc<dyn LlmProvider>,
    model: String,
    user_msg_id: Uuid,
    task_tx: mpsc::UnboundedSender<TaskMessage>,
) {
    tokio::spawn(async move {
        let phase_id = Uuid::new_v4();
        send(
            &task_tx,
            AgentEvent::Progress {
                phase_id,
                phase: AgentPhase::CountingTokens,
            },
        );
        // Read the user message content under lock, then release before async API call.
        let content = {
            let g = graph.read();
            if let Some(Node::Message { content, .. }) = g.node(user_msg_id) {
                Some(content.clone())
            } else {
                None
            }
        };
        if let Some(content) = content {
            let msg = vec![ChatMessage::text("user", &content)];
            if let Ok(count) = provider.count_tokens(&msg, &model, None, &[]).await {
                send(
                    &task_tx,
                    AgentEvent::UserTokensCounted {
                        node_id: user_msg_id,
                        count,
                    },
                );
            }
        }
        send(&task_tx, AgentEvent::PhaseCompleted { phase_id });
    });
}

/// Add the assistant response and think block to the shared graph,
/// send `IterationCommitted` notification.
fn apply_iteration_to_graph(
    graph: &SharedGraph,
    result: &StreamResult,
    config: &ChatConfig,
    task_tx: &mpsc::UnboundedSender<TaskMessage>,
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

    send(
        task_tx,
        AgentEvent::IterationCommitted {
            assistant_id,
            stop_reason: result.stop_reason,
        },
    );

    Ok(assistant_id)
}

/// Add tool call nodes to the shared graph, send dispatch notifications,
/// wait for results from the main loop.
async fn dispatch_and_wait_for_tools(
    graph: &SharedGraph,
    result: &StreamResult,
    assistant_id: Uuid,
    tool_result_rx: &mut mpsc::UnboundedReceiver<AgentToolResult>,
    task_tx: &mpsc::UnboundedSender<TaskMessage>,
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

            // Notify main loop to spawn executor (graph mutation already done).
            send(
                task_tx,
                AgentEvent::ToolCallDispatched {
                    tool_call_id: record.tool_call_id,
                    arguments: args,
                },
            );
        }
    }

    let tools_phase = Uuid::new_v4();
    send(
        task_tx,
        AgentEvent::Progress {
            phase_id: tools_phase,
            phase: AgentPhase::ExecutingTools {
                count: pending_ids.len(),
            },
        },
    );

    let timed_out = wait_for_tool_results(&mut pending_ids, tool_result_rx, task_tx, graph).await;
    send(
        task_tx,
        AgentEvent::PhaseCompleted {
            phase_id: tools_phase,
        },
    );
    timed_out
}

/// Wait for all pending tool calls to complete. Returns `true` if timed out.
/// The main loop updates the shared graph for each completion. The agent just
/// tracks which calls are still pending.
async fn wait_for_tool_results(
    pending_ids: &mut HashSet<Uuid>,
    tool_result_rx: &mut mpsc::UnboundedReceiver<AgentToolResult>,
    task_tx: &mpsc::UnboundedSender<TaskMessage>,
    graph: &SharedGraph,
) -> bool {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(60);

    while !pending_ids.is_empty() {
        tokio::select! {
            Some(result) = tool_result_rx.recv() => {
                // Main loop already updated the shared graph.
                pending_ids.remove(&result.tool_call_id);
            }
            () = tokio::time::sleep_until(deadline) => {
                timeout_pending_tools(graph, pending_ids);
                send(task_tx, AgentEvent::Error("Tool call(s) timed out".into()));
                return true;
            }
        }
    }

    false
}

/// Write timeout results to the shared graph for tools that didn't complete in time.
/// Checks current status to avoid overwriting tools the main loop already completed.
fn timeout_pending_tools(graph: &SharedGraph, pending_ids: &mut HashSet<Uuid>) {
    let mut g = graph.write();
    for tc_id in pending_ids.drain() {
        // Don't overwrite if the main loop already completed this tool.
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
