use crate::graph::tool_types::{ToolCallArguments, ToolCallStatus, ToolResultContent};
use crate::graph::{BackgroundTaskKind, EdgeKind, Node, StopReason, TaskStatus};
use crate::tasks::{AgentEvent, AgentPhase, AgentToolResult, TaskMessage, ToolExtractionOutcome};
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
            }
            TaskMessage::ToolsDiscovered(tools) => {
                let mut g = self.graph.write();
                g.remove_nodes_by(|n| matches!(n, Node::Tool { .. }));
                let root_id = g.branch_leaf(g.active_branch());
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
            }
            TaskMessage::ToolExtractionComplete {
                trigger_message_id,
                result,
            } => match result {
                ToolExtractionOutcome::Plan(plan) => {
                    let mut g = self.graph.write();
                    let node = crate::tools::plan_result_to_node(&plan);
                    let node_id = g.add_node(node);
                    let _ = g.add_edge(
                        node_id,
                        trigger_message_id,
                        crate::tools::tool_result_edge_kind(),
                    );
                    self.tui_state.status_message =
                        Some(format!("Work item created: {}", plan.title));
                }
            },
            TaskMessage::ToolCallCompleted {
                tool_call_id,
                content,
                is_error,
            } => {
                // Graph mutation first (shared graph), then notify agent.
                // This ensures the graph is updated before the agent reads it.
                self.handle_tool_call_completed(tool_call_id, content, is_error);
                if let Some(tx) = &self.agent_tool_tx {
                    let _ = tx.send(AgentToolResult { tool_call_id });
                }
            }
            TaskMessage::Agent(event) => self.handle_agent_event(event),
        }
    }

    fn handle_agent_event(&mut self, event: AgentEvent) {
        match event {
            AgentEvent::Progress { phase_id, phase } => {
                self.tui_state.status_message = Some(phase.to_string());
                self.track_phase_node(phase_id, &phase);
                self.ensure_agent_display();
                match phase {
                    AgentPhase::Receiving => {
                        if let Some(ref mut d) = self.tui_state.agent_display {
                            d.phase = AgentVisualPhase::Streaming {
                                text: String::new(),
                                is_thinking: false,
                            };
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
            AgentEvent::PhaseCompleted { phase_id } => {
                self.complete_phase(phase_id);
            }
            AgentEvent::UserTokensCounted { node_id, count } => {
                self.graph.write().set_input_tokens(node_id, count);
            }
            AgentEvent::StreamDelta { text, is_thinking } => {
                if let Some(ref mut d) = self.tui_state.agent_display {
                    d.phase = AgentVisualPhase::Streaming { text, is_thinking };
                }
                if self.tui_state.scroll_mode == crate::tui::ScrollMode::Auto {
                    self.tui_state.scroll_offset = u16::MAX;
                }
            }
            AgentEvent::IterationCommitted {
                assistant_id,
                stop_reason,
            } => {
                if stop_reason == Some(StopReason::MaxTokens) {
                    self.tui_state.error_message =
                        Some("Response truncated — continuing automatically".to_string());
                }
                if let Some(ref mut d) = self.tui_state.agent_display {
                    d.iteration_node_ids.push(assistant_id);
                    if stop_reason == Some(StopReason::ToolUse) {
                        d.phase = AgentVisualPhase::ExecutingTools;
                    }
                }
            }
            AgentEvent::ToolCallDispatched {
                tool_call_id,
                arguments,
            } => {
                // Agent already added the ToolCall node to the shared graph.
                // We only spawn execution and track the cancel token.
                let token = self
                    .cancel_token
                    .as_ref()
                    .map_or_else(CancellationToken::new, CancellationToken::child_token);
                self.task_tokens.insert(tool_call_id, token.clone());
                tool_executor::spawn_tool_execution(
                    tool_call_id,
                    arguments,
                    self.task_tx.clone(),
                    token,
                );
            }
            AgentEvent::Finished => {
                self.complete_all_phases();
                self.tui_state.agent_display = None;
                self.tui_state.status_message = None;
                self.agent_tool_tx = None;
                self.cancel_token = None;
                self.task_tokens.clear();
                let _ = self.save();
            }
            AgentEvent::Error(msg) => {
                self.tui_state.error_message = Some(msg);
            }
        }
    }

    /// Mark the previous agent phase as Completed and create a new Running phase node.
    fn track_phase_node(&mut self, phase_id: Uuid, phase: &AgentPhase) {
        self.graph.write().add_node(Node::BackgroundTask {
            id: phase_id,
            kind: BackgroundTaskKind::AgentPhase,
            status: TaskStatus::Running,
            description: phase.to_string(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        });
        self.active_phase_ids.insert(phase_id);
    }

    /// Complete a specific phase. Also handles late-arriving completions where
    /// `Finished` has already drained `active_phase_ids` (e.g. fire-and-forget
    /// token counting sending `PhaseCompleted` after the agent loop ends).
    fn complete_phase(&mut self, phase_id: Uuid) {
        self.active_phase_ids.remove(&phase_id);
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

    fn complete_all_phases(&mut self) {
        let ids: Vec<Uuid> = self.active_phase_ids.drain().collect();
        let mut g = self.graph.write();
        for id in ids {
            if let Some(Node::BackgroundTask { description, .. }) = g.node(id) {
                let desc = description.clone();
                let _ = g.update_background_task_status(id, TaskStatus::Completed, desc);
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
        let token = self
            .cancel_token
            .as_ref()
            .map_or_else(CancellationToken::new, CancellationToken::child_token);
        self.task_tokens.insert(tool_call_id, token.clone());
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

        // Skip stale completions for tool calls already resolved (e.g. timed out).
        if let Some(Node::ToolCall { status, .. }) = g.node(tool_call_id) {
            if *status == ToolCallStatus::Completed || *status == ToolCallStatus::Failed {
                return;
            }
        }

        // Apply `set` config mutation for both user-triggered and LLM-triggered calls.
        if !is_error {
            if let Some(Node::ToolCall {
                arguments: ToolCallArguments::Set { key, value },
                ..
            }) = g.node(tool_call_id)
            {
                let (k, v) = (key.clone(), value.clone());
                drop(g);
                crate::tool_executor::apply_config_set(&mut self.config, &k, &v);
                g = self.graph.write();
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

        self.task_tokens.remove(&tool_call_id);
    }

    /// Cancel a running task by its graph node ID.
    pub(super) fn cancel_task(&mut self, id: Uuid) {
        if let Some(token) = self.task_tokens.remove(&id) {
            token.cancel();
        }
    }
}
