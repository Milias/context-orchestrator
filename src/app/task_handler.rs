use crate::graph::tool_types::{ToolCallArguments, ToolCallStatus, ToolResultContent};
use crate::graph::{EdgeKind, Node, Role};
use crate::tasks::{AgentEvent, AgentToolResult, TaskMessage, ToolExtractionOutcome};
use crate::tool_executor;

use chrono::Utc;
use uuid::Uuid;

use super::App;

impl App {
    pub(super) fn handle_task_message(&mut self, msg: TaskMessage) {
        match msg {
            TaskMessage::GitFilesUpdated(files) => {
                self.graph
                    .remove_nodes_by(|n| matches!(n, Node::GitFile { .. }));
                let root_id = self.graph.branch_leaf(self.graph.active_branch());
                for file in files {
                    let node = Node::GitFile {
                        id: Uuid::new_v4(),
                        path: file.path,
                        status: file.status,
                        updated_at: Utc::now(),
                    };
                    let node_id = self.graph.add_node(node);
                    if let Some(root) = root_id {
                        let _ = self.graph.add_edge(node_id, root, EdgeKind::Indexes);
                    }
                }
            }
            TaskMessage::ToolsDiscovered(tools) => {
                self.graph
                    .remove_nodes_by(|n| matches!(n, Node::Tool { .. }));
                let root_id = self.graph.branch_leaf(self.graph.active_branch());
                self.tui_state.available_tools = tools
                    .iter()
                    .map(|t| crate::tui::CompletionCandidate {
                        name: t.name.clone(),
                        description: t.description.clone(),
                    })
                    .collect();
                for tool in tools {
                    let node = Node::Tool {
                        id: Uuid::new_v4(),
                        name: tool.name,
                        description: tool.description,
                        updated_at: Utc::now(),
                    };
                    let node_id = self.graph.add_node(node);
                    if let Some(root) = root_id {
                        let _ = self.graph.add_edge(node_id, root, EdgeKind::Provides);
                    }
                }
            }
            TaskMessage::TaskStatusChanged {
                task_id,
                kind,
                status,
                description,
            } => {
                self.graph.upsert_node(Node::BackgroundTask {
                    id: task_id,
                    kind,
                    status,
                    description,
                    created_at: Utc::now(),
                    updated_at: Utc::now(),
                });
            }
            TaskMessage::ToolExtractionComplete {
                trigger_message_id,
                result,
            } => match result {
                ToolExtractionOutcome::Plan(plan) => {
                    let node = crate::tools::plan_result_to_node(&plan);
                    let node_id = self.graph.add_node(node);
                    let _ = self.graph.add_edge(
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
                // Forward to running agent before applying to graph
                if let Some(tx) = &self.agent_tool_tx {
                    let _ = tx.send(AgentToolResult {
                        tool_call_id,
                        content: content.clone(),
                        is_error,
                    });
                }
                self.handle_tool_call_completed(tool_call_id, content, is_error);
            }
            TaskMessage::Agent(event) => self.handle_agent_event(event),
        }
    }

    fn handle_agent_event(&mut self, event: AgentEvent) {
        match event {
            AgentEvent::Progress(phase) => {
                self.tui_state.status_message = Some(phase.to_string());
            }
            AgentEvent::UserTokensCounted { node_id, count } => {
                self.graph.set_input_tokens(node_id, count);
            }
            AgentEvent::StreamDelta { text, is_thinking } => {
                self.tui_state.streaming_response = Some(text);
                if is_thinking {
                    self.tui_state.status_message = Some("Thinking...".to_string());
                }
                if self.tui_state.auto_scroll {
                    self.tui_state.scroll_offset = u16::MAX;
                }
            }
            AgentEvent::IterationDone {
                response,
                think_text,
                output_tokens,
                stop_reason,
            } => {
                self.tui_state.streaming_response = None;

                let leaf = self
                    .graph
                    .branch_leaf(self.graph.active_branch())
                    .expect("No leaf node for active branch");
                let assistant_id = Uuid::new_v4();
                let assistant_node = Node::Message {
                    id: assistant_id,
                    role: Role::Assistant,
                    content: response,
                    created_at: Utc::now(),
                    model: Some(self.config.anthropic_model.clone()),
                    input_tokens: None,
                    output_tokens,
                };
                let _ = self.graph.add_message(leaf, assistant_node);

                if !think_text.is_empty() {
                    let think_node = Node::ThinkBlock {
                        id: Uuid::new_v4(),
                        content: think_text,
                        parent_message_id: assistant_id,
                        created_at: Utc::now(),
                    };
                    let think_id = self.graph.add_node(think_node);
                    let _ = self
                        .graph
                        .add_edge(think_id, assistant_id, EdgeKind::ThinkingOf);
                }

                // Invalidate render cache for the new message
                self.tui_state.render_cache.remove(&assistant_id);

                if stop_reason.as_deref() == Some("tool_use") {
                    self.tui_state.streaming_response = Some(String::new());
                }
            }
            AgentEvent::ToolCallRequest {
                tool_call_id,
                assistant_id,
                api_id,
                name,
                input,
            } => {
                let args = crate::graph::parse_tool_arguments(&name, &input);
                self.handle_tool_call_dispatched(tool_call_id, assistant_id, args, Some(api_id));
            }
            AgentEvent::Finished => {
                self.tui_state.agent_running = false;
                self.tui_state.streaming_response = None;
                self.tui_state.status_message = None;
                self.agent_tool_tx = None;
                self.cancel_tx = None;
                let _ = self.save();
            }
            AgentEvent::Error(msg) => {
                self.tui_state.error_message = Some(msg);
            }
        }
    }

    pub(super) fn handle_tool_call_dispatched(
        &mut self,
        tool_call_id: Uuid,
        parent_message_id: Uuid,
        arguments: ToolCallArguments,
        api_tool_use_id: Option<String>,
    ) {
        let tool_call = Node::ToolCall {
            id: tool_call_id,
            api_tool_use_id,
            arguments: arguments.clone(),
            status: ToolCallStatus::Pending,
            parent_message_id,
            created_at: Utc::now(),
            completed_at: None,
        };
        self.graph.add_node(tool_call);
        let _ = self
            .graph
            .add_edge(tool_call_id, parent_message_id, EdgeKind::Invoked);
        let _ = self
            .graph
            .update_tool_call_status(tool_call_id, ToolCallStatus::Running, None);

        tool_executor::spawn_tool_execution(tool_call_id, arguments, self.task_tx.clone());
    }

    pub(super) fn handle_tool_call_completed(
        &mut self,
        tool_call_id: Uuid,
        content: ToolResultContent,
        is_error: bool,
    ) {
        // Skip stale completions for tool calls already resolved (e.g. timed out).
        if let Some(Node::ToolCall { status, .. }) = self.graph.node(tool_call_id) {
            if *status == ToolCallStatus::Completed || *status == ToolCallStatus::Failed {
                return;
            }
        }

        let new_status = if is_error {
            ToolCallStatus::Failed
        } else {
            ToolCallStatus::Completed
        };
        let _ = self
            .graph
            .update_tool_call_status(tool_call_id, new_status, Some(Utc::now()));

        let result_id = Uuid::new_v4();
        let result_node = Node::ToolResult {
            id: result_id,
            tool_call_id,
            content,
            is_error,
            created_at: Utc::now(),
        };
        self.graph.add_node(result_node);
        let _ = self
            .graph
            .add_edge(result_id, tool_call_id, EdgeKind::Produced);
    }
}
