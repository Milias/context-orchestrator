use crate::graph::tool_types::{ToolCallArguments, ToolCallStatus};
use crate::graph::{EdgeKind, Node};
use crate::tasks::{TaskMessage, ToolExtractionOutcome};
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
            TaskMessage::ToolCallDispatched {
                tool_call_id,
                parent_message_id,
                arguments,
            } => {
                self.handle_tool_call_dispatched(tool_call_id, parent_message_id, arguments, None);
            }
            TaskMessage::ToolCallCompleted {
                tool_call_id,
                content,
                is_error,
            } => {
                self.handle_tool_call_completed(tool_call_id, content, is_error);
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
        content: String,
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
