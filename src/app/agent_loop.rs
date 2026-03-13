use crate::graph::tool_types::{ToolCallStatus, ToolResultContent};
use crate::graph::{parse_tool_arguments, ConversationGraph, EdgeKind, Node, Role};
use crate::llm::{ChatConfig, ChatMessage, LlmProvider, ToolDefinition};
use crate::tasks::{AgentEvent, AgentPhase, AgentToolResult, TaskMessage};

use super::agent_streaming::{self, send, StreamResult};
use super::context;

use chrono::Utc;
use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, watch};
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
    graph: ConversationGraph,
    provider: Arc<dyn LlmProvider>,
    config: AgentLoopConfig,
    user_msg_id: Uuid,
    task_tx: mpsc::UnboundedSender<TaskMessage>,
    tool_result_rx: mpsc::UnboundedReceiver<AgentToolResult>,
    cancel_rx: watch::Receiver<bool>,
) {
    tokio::spawn(async move {
        let result = run_agent_loop(
            graph,
            provider,
            config,
            user_msg_id,
            &task_tx,
            tool_result_rx,
            cancel_rx,
        )
        .await;
        if let Err(e) = result {
            let _ = task_tx.send(TaskMessage::Agent(AgentEvent::Error(e.to_string())));
        }
        let _ = task_tx.send(TaskMessage::Agent(AgentEvent::Finished));
    });
}

async fn run_agent_loop(
    mut graph: ConversationGraph,
    provider: Arc<dyn LlmProvider>,
    config: AgentLoopConfig,
    user_msg_id: Uuid,
    task_tx: &mpsc::UnboundedSender<TaskMessage>,
    mut tool_result_rx: mpsc::UnboundedReceiver<AgentToolResult>,
    cancel_rx: watch::Receiver<bool>,
) -> anyhow::Result<()> {
    count_user_tokens(&graph, &provider, &config, user_msg_id, task_tx).await;

    let chat_config = ChatConfig {
        model: config.model.clone(),
        max_tokens: config.max_tokens,
        system_prompt: None,
        tools: config.tools.clone(),
    };

    for _ in 0..config.max_tool_loop_iterations {
        send(task_tx, AgentEvent::Progress(AgentPhase::BuildingContext));
        let (system_prompt, messages) = context::build_context(
            &graph,
            provider.as_ref(),
            &config.model,
            config.max_context_tokens,
            &config.tools,
        )
        .await?;

        let loop_config = ChatConfig {
            system_prompt,
            ..chat_config.clone()
        };

        let result = agent_streaming::stream_llm_response(
            &provider,
            messages,
            &loop_config,
            task_tx,
            &cancel_rx,
        )
        .await?;

        if result.cancelled || (result.response.is_empty() && result.tool_use_records.is_empty()) {
            break;
        }

        let assistant_id = apply_iteration_to_graph(&mut graph, &result, &loop_config, task_tx)?;

        let is_tool_use = result.stop_reason.as_deref() == Some("tool_use")
            && !result.tool_use_records.is_empty();

        if !is_tool_use {
            break;
        }

        let timed_out = dispatch_and_wait_for_tools(
            &mut graph,
            &result,
            assistant_id,
            &mut tool_result_rx,
            task_tx,
        )
        .await;

        if timed_out {
            break;
        }
    }

    Ok(())
}

/// Count user message tokens and notify the main loop.
async fn count_user_tokens(
    graph: &ConversationGraph,
    provider: &Arc<dyn LlmProvider>,
    config: &AgentLoopConfig,
    user_msg_id: Uuid,
    task_tx: &mpsc::UnboundedSender<TaskMessage>,
) {
    send(task_tx, AgentEvent::Progress(AgentPhase::CountingTokens));
    if let Some(Node::Message { content, .. }) = graph.node(user_msg_id) {
        let msg = vec![ChatMessage::text("user", content)];
        if let Ok(count) = provider.count_tokens(&msg, &config.model, None, &[]).await {
            send(
                task_tx,
                AgentEvent::UserTokensCounted {
                    node_id: user_msg_id,
                    count,
                },
            );
        }
    }
}

/// Add the assistant response and think block to the local graph, send `IterationDone`.
fn apply_iteration_to_graph(
    graph: &mut ConversationGraph,
    result: &StreamResult,
    config: &ChatConfig,
    task_tx: &mpsc::UnboundedSender<TaskMessage>,
) -> anyhow::Result<Uuid> {
    let assistant_id = Uuid::new_v4();
    send(
        task_tx,
        AgentEvent::IterationDone {
            response: result.response.clone(),
            think_text: result.think_text.clone(),
            output_tokens: result.output_tokens,
            stop_reason: result.stop_reason.clone(),
        },
    );

    let leaf = graph
        .branch_leaf(graph.active_branch())
        .ok_or_else(|| anyhow::anyhow!("No leaf node for active branch"))?;
    let assistant_node = Node::Message {
        id: assistant_id,
        role: Role::Assistant,
        content: result.response.clone(),
        created_at: Utc::now(),
        model: Some(config.model.clone()),
        input_tokens: None,
        output_tokens: result.output_tokens,
    };
    graph.add_message(leaf, assistant_node)?;

    if !result.think_text.is_empty() {
        let think_node = Node::ThinkBlock {
            id: Uuid::new_v4(),
            content: result.think_text.clone(),
            parent_message_id: assistant_id,
            created_at: Utc::now(),
        };
        let think_id = graph.add_node(think_node);
        graph.add_edge(think_id, assistant_id, EdgeKind::ThinkingOf)?;
    }

    Ok(assistant_id)
}

/// Send tool call requests to main loop, add them to local graph, wait for results.
async fn dispatch_and_wait_for_tools(
    graph: &mut ConversationGraph,
    result: &StreamResult,
    assistant_id: Uuid,
    tool_result_rx: &mut mpsc::UnboundedReceiver<AgentToolResult>,
    task_tx: &mpsc::UnboundedSender<TaskMessage>,
) -> bool {
    let mut pending_ids = HashSet::new();
    for record in &result.tool_use_records {
        send(
            task_tx,
            AgentEvent::ToolCallRequest {
                tool_call_id: record.tool_call_id,
                assistant_id,
                api_id: record.api_id.clone(),
                name: record.name.clone(),
                input: record.input.clone(),
            },
        );
        pending_ids.insert(record.tool_call_id);

        let args = parse_tool_arguments(&record.name, &record.input);
        let tool_call = Node::ToolCall {
            id: record.tool_call_id,
            api_tool_use_id: Some(record.api_id.clone()),
            arguments: args,
            status: ToolCallStatus::Running,
            parent_message_id: assistant_id,
            created_at: Utc::now(),
            completed_at: None,
        };
        graph.add_node(tool_call);
        let _ = graph.add_edge(record.tool_call_id, assistant_id, EdgeKind::Invoked);
    }

    send(
        task_tx,
        AgentEvent::Progress(AgentPhase::ExecutingTools {
            count: pending_ids.len(),
        }),
    );

    wait_for_tool_results(graph, &mut pending_ids, tool_result_rx, task_tx).await
}

/// Wait for all pending tool calls to complete. Returns `true` if timed out.
async fn wait_for_tool_results(
    graph: &mut ConversationGraph,
    pending_ids: &mut HashSet<Uuid>,
    tool_result_rx: &mut mpsc::UnboundedReceiver<AgentToolResult>,
    task_tx: &mpsc::UnboundedSender<TaskMessage>,
) -> bool {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(60);

    while !pending_ids.is_empty() {
        tokio::select! {
            Some(result) = tool_result_rx.recv() => {
                apply_tool_result(graph, &result);
                pending_ids.remove(&result.tool_call_id);
                if !pending_ids.is_empty() {
                    send(task_tx, AgentEvent::Progress(AgentPhase::ExecutingTools {
                        count: pending_ids.len(),
                    }));
                }
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

fn apply_tool_result(graph: &mut ConversationGraph, result: &AgentToolResult) {
    let status = if result.is_error {
        ToolCallStatus::Failed
    } else {
        ToolCallStatus::Completed
    };
    let _ = graph.update_tool_call_status(result.tool_call_id, status, Some(Utc::now()));
    let result_id = Uuid::new_v4();
    let result_node = Node::ToolResult {
        id: result_id,
        tool_call_id: result.tool_call_id,
        content: result.content.clone(),
        is_error: result.is_error,
        created_at: Utc::now(),
    };
    graph.add_node(result_node);
    let _ = graph.add_edge(result_id, result.tool_call_id, EdgeKind::Produced);
}

fn timeout_pending_tools(graph: &mut ConversationGraph, pending_ids: &mut HashSet<Uuid>) {
    for tc_id in pending_ids.drain() {
        let _ = graph.update_tool_call_status(tc_id, ToolCallStatus::Failed, Some(Utc::now()));
        let result_id = Uuid::new_v4();
        let result_node = Node::ToolResult {
            id: result_id,
            tool_call_id: tc_id,
            content: ToolResultContent::text("Tool execution timed out"),
            is_error: true,
            created_at: Utc::now(),
        };
        graph.add_node(result_node);
        let _ = graph.add_edge(result_id, tc_id, EdgeKind::Produced);
    }
}
