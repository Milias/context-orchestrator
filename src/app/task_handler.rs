use crate::graph::{EdgeKind, Node};
use crate::tasks::{TaskMessage, ToolExtractionOutcome};

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
        }
    }
}
