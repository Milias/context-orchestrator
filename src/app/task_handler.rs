use crate::graph::event::GraphEvent;
use crate::graph::tool_types::{ToolCallArguments, ToolCallStatus, ToolResultContent};
use crate::graph::{BackgroundTaskKind, EdgeKind, Node, TaskStatus};
use crate::storage::{TokenDirection, TokenEvent};
use crate::tasks::{AgentEvent, AgentPhase, TaskMessage};
use crate::tool_executor;

use chrono::Utc;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use super::App;

impl App {
    pub(super) fn handle_task_message(&mut self, msg: TaskMessage) {
        match msg {
            TaskMessage::GitFilesUpdated(files) => {
                let mut g = self.graph.write();
                g.remove_nodes_by(|n| matches!(n, Node::GitFile { .. }));
                let root_id = g.branch_leaf(g.active_branch());
                let count = files.len();
                for file in files {
                    let node = Node::GitFile {
                        id: Uuid::new_v4(),
                        path: file.path,
                        status: file.status,
                        updated_at: Utc::now(),
                    };
                    let node_id = g.add_node(node);
                    if let Some(root) = root_id {
                        let _ = g.add_edge(node_id, root, EdgeKind::Indexes);
                    }
                }
                g.emit(GraphEvent::GitFilesRefreshed { count });
            }
            TaskMessage::ToolsDiscovered(tools) => {
                let mut g = self.graph.write();
                g.remove_nodes_by(|n| matches!(n, Node::Tool { .. }));
                let root_id = g.branch_leaf(g.active_branch());
                let count = tools.len();
                for tool in tools {
                    let node = Node::Tool {
                        id: Uuid::new_v4(),
                        name: tool.name,
                        description: tool.description,
                        updated_at: Utc::now(),
                    };
                    let node_id = g.add_node(node);
                    if let Some(root) = root_id {
                        let _ = g.add_edge(node_id, root, EdgeKind::Provides);
                    }
                }
                g.emit(GraphEvent::ToolsRefreshed { count });
            }
            TaskMessage::TaskStatusChanged {
                task_id,
                kind,
                status,
                description,
            } => {
                let mut g = self.graph.write();
                if g.node(task_id).is_some() {
                    let _ = g.update_background_task_status(task_id, status, description);
                } else {
                    g.add_node(Node::BackgroundTask {
                        id: task_id,
                        kind,
                        status,
                        description,
                        created_at: Utc::now(),
                        updated_at: Utc::now(),
                    });
                }
                g.emit(GraphEvent::BackgroundTaskChanged {
                    node_id: task_id,
                    status,
                });
            }
            TaskMessage::ToolCallCompleted {
                tool_call_id,
                content,
                is_error,
            } => {
                // Graph mutation first, then route to the owning agent.
                self.handle_tool_call_completed(tool_call_id, content, is_error);
                self.agents.route_tool_result(tool_call_id);
            }
            TaskMessage::Agent { agent_id, event } => {
                self.handle_agent_event(agent_id, event);
            }
            // Analytics events flow through the EventBus — no direct TUI mutations.
            TaskMessage::TokenTotalsUpdated(totals) => {
                self.graph.read().emit(GraphEvent::TokenTotalsUpdated {
                    input: totals.input,
                    output: totals.output,
                });
            }
            TaskMessage::AnalyticsError(msg) => {
                self.graph.read().emit(GraphEvent::ErrorOccurred {
                    message: format!("Analytics: {msg}"),
                });
            }
        }
    }

    fn handle_agent_event(&mut self, agent_id: Uuid, event: AgentEvent) {
        match event {
            AgentEvent::Progress { phase_id, phase } => {
                self.agents.track_phase(agent_id, phase_id);
                self.track_phase_node(phase_id, &phase);
                // TUI update flows through EventBus.
                self.graph
                    .read()
                    .emit(GraphEvent::AgentPhaseChanged { agent_id, phase });
            }
            AgentEvent::PhaseCompleted { phase_id } => {
                self.agents.complete_phase(agent_id, &phase_id);
                self.complete_phase(phase_id);
            }
            AgentEvent::StreamDelta { text, is_thinking } => {
                // TUI update flows through EventBus.
                self.graph.read().emit(GraphEvent::StreamDelta {
                    agent_id,
                    text,
                    is_thinking,
                });
            }
            AgentEvent::IterationCommitted {
                assistant_id,
                stop_reason,
            } => {
                // Record output tokens (graph operation, not TUI).
                if let Some(tokens) = self
                    .graph
                    .read()
                    .node(assistant_id)
                    .and_then(Node::output_tokens)
                {
                    self.spawn_token_record(TokenEvent {
                        conversation_id: self.metadata.id.clone(),
                        direction: TokenDirection::Output,
                        tokens,
                        model: Some(self.config.anthropic_model.clone()),
                    });
                }
                // TUI update flows through EventBus.
                self.graph.read().emit(GraphEvent::AgentIterationCommitted {
                    agent_id,
                    assistant_id,
                    stop_reason,
                });
            }
            AgentEvent::ToolCallDispatched {
                tool_call_id,
                arguments,
            } => {
                let token = self.agents.child_cancel_token(agent_id);
                self.agents
                    .track_tool_call(agent_id, tool_call_id, token.clone());
                tool_executor::spawn_tool_execution(
                    tool_call_id,
                    arguments,
                    self.task_tx.clone(),
                    token,
                );
            }
            AgentEvent::Idle => {
                // TUI update flows through EventBus — clears agent display.
                self.graph.read().emit(GraphEvent::AgentIdle { agent_id });
            }
            AgentEvent::Finished => {
                let phase_ids = self.agents.drain_phases(agent_id);
                for pid in phase_ids {
                    self.complete_phase(pid);
                }
                self.agents.remove(agent_id);
                {
                    let mut g = self.graph.write();
                    // Release claims held by this agent.
                    g.edges
                        .retain(|e| !(e.kind == EdgeKind::ClaimedBy && e.to == agent_id));
                    // Safety net: clean up any stale ApiError nodes.
                    g.remove_nodes_by(|n| matches!(n, Node::ApiError { .. }));
                }
                let _ = self.save();
                // TUI update flows through EventBus.
                self.graph
                    .read()
                    .emit(GraphEvent::AgentFinished { agent_id });
            }
            AgentEvent::ApiError { phase_id, message } => {
                // Record phase failure — do NOT cancel the agent.
                // Error node is recorded synchronously in the agent loop (no race).
                self.fail_phase(phase_id, &message);
                self.graph
                    .read()
                    .emit(GraphEvent::ErrorOccurred { message });
            }
            AgentEvent::StatusMessage(msg) => {
                // TUI display only — do NOT cancel the agent.
                self.graph
                    .read()
                    .emit(GraphEvent::ErrorOccurred { message: msg });
            }
            AgentEvent::Error(msg) => {
                // Fatal error — cancel the agent.
                self.agents.cancel_agent(agent_id);
                self.graph
                    .read()
                    .emit(GraphEvent::ErrorOccurred { message: msg });
            }
        }
    }

    /// Create a `BackgroundTask` node for a phase. Per-agent tracking is in the registry.
    fn track_phase_node(&mut self, phase_id: Uuid, phase: &AgentPhase) {
        self.graph.write().add_node(Node::BackgroundTask {
            id: phase_id,
            kind: BackgroundTaskKind::AgentPhase,
            status: TaskStatus::Running,
            description: phase.to_string(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        });
    }

    /// Mark a phase as Failed in the graph with an error description.
    fn fail_phase(&mut self, phase_id: Uuid, error_msg: &str) {
        let mut g = self.graph.write();
        if let Some(Node::BackgroundTask { status, .. }) = g.node(phase_id) {
            if *status == TaskStatus::Running {
                let desc = format!("API Error: {error_msg}");
                let _ = g.update_background_task_status(phase_id, TaskStatus::Failed, desc);
            }
        }
    }

    /// Mark a phase as Completed in the graph.
    fn complete_phase(&mut self, phase_id: Uuid) {
        let mut g = self.graph.write();
        if let Some(Node::BackgroundTask {
            status,
            description,
            ..
        }) = g.node(phase_id)
        {
            if *status == TaskStatus::Running {
                let desc = description.clone();
                let _ = g.update_background_task_status(phase_id, TaskStatus::Completed, desc);
            }
        }
    }

    /// Dispatch a user-triggered tool call: add to graph, spawn execution, track cancel.
    /// Only used for user triggers (via `/tool_name args`). Agent tool calls
    /// are added to the shared graph by the agent loop directly.
    pub(super) fn handle_tool_call_dispatched(
        &mut self,
        tool_call_id: Uuid,
        parent_message_id: Uuid,
        arguments: ToolCallArguments,
        api_tool_use_id: Option<String>,
    ) {
        self.graph.write().add_tool_call(
            tool_call_id,
            parent_message_id,
            arguments.clone(),
            api_tool_use_id,
        );
        // User-triggered tool calls have no owning agent — use a standalone token.
        let token = CancellationToken::new();
        tool_executor::spawn_tool_execution(tool_call_id, arguments, self.task_tx.clone(), token);
    }

    /// Handle tool completion: update graph status, add result, apply side-effects.
    /// Handles both user-triggered and agent-triggered tool calls.
    pub(super) fn handle_tool_call_completed(
        &mut self,
        tool_call_id: Uuid,
        content: ToolResultContent,
        is_error: bool,
    ) {
        let mut g = self.graph.write();
        let mut content = content;

        // Skip stale completions for tool calls already resolved (e.g. timed out).
        if let Some(Node::ToolCall { status, .. }) = g.node(tool_call_id) {
            if *status == ToolCallStatus::Completed || *status == ToolCallStatus::Failed {
                return;
            }
        }

        // Apply side-effects for specific tools (both user-triggered and LLM-triggered).
        if !is_error {
            // Set: config mutation (requires dropping the graph lock).
            if let Some(Node::ToolCall {
                arguments: ToolCallArguments::Set { key, value },
                ..
            }) = g.node(tool_call_id)
            {
                let (k, v) = (key.clone(), value.clone());
                drop(g);
                if let Ok(config_key) = k.parse::<crate::tool_executor::ConfigKey>() {
                    crate::tool_executor::apply_config_set(&mut self.config, config_key, &v);
                }
                g = self.graph.write();
                if let Some(Node::ToolCall { status, .. }) = g.node(tool_call_id) {
                    if *status == ToolCallStatus::Completed || *status == ToolCallStatus::Failed {
                        return;
                    }
                }
            }

            // Plan tools: create WorkItems, edges, enrich content with UUIDs.
            // TUI notifications flow through GraphEvent, not direct calls.
            if let Some(enriched) = super::plan::effects::apply(&mut g, tool_call_id) {
                content = enriched;
            }

            // Q/A tools: create Question nodes + edges. Routing via EventBus.
            if let Some(enriched) = super::qa::effects::apply(&mut g, tool_call_id) {
                content = enriched;
            }
        }

        let new_status = if is_error {
            ToolCallStatus::Failed
        } else {
            ToolCallStatus::Completed
        };
        let _ = g.update_tool_call_status(tool_call_id, new_status, Some(Utc::now()));
        g.add_tool_result(tool_call_id, content, is_error);
        drop(g);
    }

    /// Spawn a background task to record a token event and refresh lifetime totals.
    ///
    /// The write + query runs on the `tokio_rusqlite` background thread.
    /// Fresh totals are sent back via [`TaskMessage::TokenTotalsUpdated`],
    /// which triggers the animated counter update via the `EventBus`.
    fn spawn_token_record(&self, event: TokenEvent) {
        let Some(store) = self.token_store.clone() else {
            return;
        };
        let tx = self.task_tx.clone();

        tokio::spawn(async move {
            if let Err(e) = store.record(&event).await {
                let _ = tx.send(TaskMessage::AnalyticsError(format!("{e}")));
                return;
            }
            match store.lifetime_totals().await {
                Ok(totals) => {
                    let _ = tx.send(TaskMessage::TokenTotalsUpdated(totals));
                }
                Err(e) => {
                    let _ = tx.send(TaskMessage::AnalyticsError(format!("{e}")));
                }
            }
        });
    }
}
