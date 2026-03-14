use crate::graph::tool_types::{ToolCallArguments, ToolCallStatus, ToolResultContent};
use crate::graph::{BackgroundTaskKind, EdgeKind, Node, StopReason, TaskStatus};
use crate::storage::{TokenDirection, TokenEvent};
use crate::tasks::{AgentEvent, AgentPhase, TaskMessage};
use crate::tool_executor;
use crate::tui::{AgentDisplayState, AgentVisualPhase};

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
                g.emit(crate::graph::event::GraphEvent::GitFilesRefreshed { count });
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
                g.emit(crate::graph::event::GraphEvent::ToolsRefreshed { count });
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
                g.emit(crate::graph::event::GraphEvent::BackgroundTaskChanged {
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
            TaskMessage::TokenTotalsUpdated(totals) => {
                self.tui_state.token_usage.input.target = totals.input;
                self.tui_state.token_usage.output.target = totals.output;
            }
            TaskMessage::AnalyticsError(msg) => {
                self.tui_state.error_message = Some(format!("Analytics: {msg}"));
            }
        }
    }

    fn handle_agent_event(&mut self, agent_id: Uuid, event: AgentEvent) {
        let is_primary = self.agents.is_primary(agent_id);
        match event {
            AgentEvent::Progress { phase_id, phase } => {
                self.agents.track_phase(agent_id, phase_id);
                self.track_phase_node(phase_id, &phase);
                if is_primary {
                    self.tui_state.status_message = Some(phase.to_string());
                    self.ensure_agent_display();
                    self.update_visual_phase(&phase);
                }
            }
            AgentEvent::PhaseCompleted { phase_id } => {
                self.agents.complete_phase(agent_id, &phase_id);
                self.complete_phase(phase_id);
            }
            AgentEvent::UserTokensCounted { node_id, count } => {
                self.graph.write().set_input_tokens(node_id, count);
                self.spawn_token_record(TokenEvent {
                    conversation_id: self.metadata.id.clone(),
                    direction: TokenDirection::Input,
                    tokens: count,
                    model: None,
                });
            }
            AgentEvent::StreamDelta { text, is_thinking } => {
                if is_primary {
                    if let Some(ref mut d) = self.tui_state.agent_display {
                        d.phase = AgentVisualPhase::Streaming { text, is_thinking };
                    }
                    if self.tui_state.scroll_mode == crate::tui::ScrollMode::Auto {
                        self.tui_state.scroll_offset = u16::MAX;
                    }
                }
            }
            AgentEvent::IterationCommitted {
                assistant_id,
                stop_reason,
            } => {
                if is_primary {
                    self.handle_iteration_committed(assistant_id, stop_reason);
                }
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
            AgentEvent::Finished => {
                let phase_ids = self.agents.drain_phases(agent_id);
                for pid in phase_ids {
                    self.complete_phase(pid);
                }
                self.agents.remove(agent_id);
                if is_primary {
                    self.tui_state.agent_display = None;
                    self.tui_state.status_message = None;
                }
                // Release claims held by this agent (identified by ClaimedBy edge target).
                self.graph
                    .write()
                    .edges
                    .retain(|e| !(e.kind == crate::graph::EdgeKind::ClaimedBy && e.to == agent_id));
                let _ = self.save();
                self.check_ready_work();
            }
            AgentEvent::Error(msg) => {
                // On error, cancel the agent's remaining work.
                self.agents.cancel_agent(agent_id);
                if is_primary {
                    self.tui_state.error_message = Some(msg);
                }
            }
        }
    }

    /// Update the TUI visual phase indicator for the primary agent.
    fn update_visual_phase(&mut self, phase: &AgentPhase) {
        match phase {
            AgentPhase::Receiving => {
                if let Some(ref mut d) = self.tui_state.agent_display {
                    d.phase = AgentVisualPhase::Streaming {
                        text: String::new(),
                        is_thinking: false,
                    };
                    d.revealed_chars = 0;
                }
            }
            AgentPhase::ExecutingTools { .. } => {
                if let Some(ref mut d) = self.tui_state.agent_display {
                    d.phase = AgentVisualPhase::ExecutingTools;
                }
            }
            AgentPhase::CountingTokens
            | AgentPhase::BuildingContext
            | AgentPhase::Connecting { .. } => {
                if let Some(ref mut d) = self.tui_state.agent_display {
                    if !matches!(d.phase, AgentVisualPhase::Streaming { .. }) {
                        d.phase = AgentVisualPhase::Preparing;
                    }
                }
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

    /// Ensure `agent_display` exists (create with Preparing phase if missing).
    fn ensure_agent_display(&mut self) {
        if self.tui_state.agent_display.is_none() {
            self.tui_state.agent_display = Some(AgentDisplayState::default());
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

    /// Process an assistant iteration committed by the agent loop.
    fn handle_iteration_committed(&mut self, assistant_id: Uuid, stop_reason: Option<StopReason>) {
        if stop_reason == Some(StopReason::MaxTokens) {
            self.tui_state.error_message =
                Some("Response truncated — continuing automatically".to_string());
        }
        if let Some(ref mut d) = self.tui_state.agent_display {
            // Snap reveal to full before committing — no trailing animation
            d.revealed_chars = usize::MAX;
            d.iteration_node_ids.push(assistant_id);
            if stop_reason == Some(StopReason::ToolUse) {
                d.phase = AgentVisualPhase::ExecutingTools;
            }
        }
        // Record output tokens from the assistant message the agent just committed.
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
    }

    /// Spawn a background task to record a token event and refresh lifetime totals.
    ///
    /// The write + query runs on the `tokio_rusqlite` background thread.
    /// Fresh totals are sent back via [`TaskMessage::TokenTotalsUpdated`],
    /// which triggers the animated counter update in the status bar.
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

    /// Cancel a running task by its `tool_call_id`.
    pub(super) fn cancel_task(&mut self, id: Uuid) {
        self.agents.cancel_tool(id);
    }
}
