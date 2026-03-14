use chrono::{DateTime, Utc};
use uuid::Uuid;

use super::history::NodeSnapshot;
use super::node::Node;
use super::tool_types::{ToolCallStatus, ToolResultContent};
use super::{ConversationGraph, EdgeKind, TaskStatus};

impl ConversationGraph {
    /// Snapshot the current node state, then apply a mutation closure.
    /// If the closure returns `Err`, the snapshot is discarded (not pushed).
    fn mutate_node<F>(&mut self, id: Uuid, mutate: F) -> anyhow::Result<()>
    where
        F: FnOnce(&mut Node) -> anyhow::Result<()>,
    {
        let node = self
            .nodes
            .get(&id)
            .ok_or_else(|| anyhow::anyhow!("Node {id} not found"))?;
        let snapshot = NodeSnapshot {
            node: node.clone(),
            captured_at: Utc::now(),
        };
        let node = self.nodes.get_mut(&id).expect("checked above");
        mutate(node)?;
        self.history.entry(id).or_default().push(snapshot);
        Ok(())
    }

    /// Update the status (and optionally `completed_at`) of a `ToolCall` node.
    /// Captures a version snapshot before the mutation.
    pub fn update_tool_call_status(
        &mut self,
        id: Uuid,
        new_status: ToolCallStatus,
        completed_at: Option<DateTime<Utc>>,
    ) -> anyhow::Result<()> {
        self.mutate_node(id, |node| match node {
            Node::ToolCall {
                status,
                completed_at: ca,
                ..
            } => {
                *status = new_status;
                *ca = completed_at;
                Ok(())
            }
            _ => anyhow::bail!("Node {id} is not a ToolCall"),
        })
    }

    /// Update the status, description, and `updated_at` of a `BackgroundTask`.
    /// Captures a version snapshot before the mutation. Preserves `created_at`.
    pub fn update_background_task_status(
        &mut self,
        id: Uuid,
        new_status: TaskStatus,
        new_description: String,
    ) -> anyhow::Result<()> {
        self.mutate_node(id, |node| match node {
            Node::BackgroundTask {
                status,
                description,
                updated_at,
                ..
            } => {
                *status = new_status;
                *description = new_description;
                *updated_at = Utc::now();
                Ok(())
            }
            _ => anyhow::bail!("Node {id} is not a BackgroundTask"),
        })
    }

    /// Set the `input_tokens` field on a `Message` node.
    /// Captures a version snapshot before the mutation.
    pub fn set_input_tokens(&mut self, node_id: Uuid, tokens: u32) {
        let _ = self.mutate_node(node_id, |node| match node {
            Node::Message { input_tokens, .. } => {
                *input_tokens = Some(tokens);
                Ok(())
            }
            _ => anyhow::bail!("Node {node_id} is not a Message"),
        });
    }

    /// Mark all `Running`/`Pending` background tasks as `Failed`.
    /// Called on startup — any still-running tasks survived a crash.
    pub fn expire_stale_tasks(&mut self) {
        self.transition_running_tasks(TaskStatus::Failed);
    }

    /// Mark all `Running`/`Pending` background tasks as `Stopped`.
    /// Called on graceful shutdown.
    pub fn stop_running_tasks(&mut self) {
        self.transition_running_tasks(TaskStatus::Stopped);
    }

    /// Transition all running/pending background tasks to a target status.
    /// Captures a version snapshot for each transitioned node.
    fn transition_running_tasks(&mut self, new_status: TaskStatus) {
        let ids: Vec<Uuid> = self
            .nodes
            .iter()
            .filter_map(|(&id, node)| {
                if let Node::BackgroundTask { status, .. } = node {
                    if matches!(status, TaskStatus::Running | TaskStatus::Pending) {
                        return Some(id);
                    }
                }
                None
            })
            .collect();

        for id in ids {
            let _ = self.mutate_node(id, |node| {
                if let Node::BackgroundTask {
                    status, updated_at, ..
                } = node
                {
                    *status = new_status;
                    *updated_at = Utc::now();
                }
                Ok(())
            });
        }
    }

    /// Remove all nodes (and their edges + history) matching a predicate.
    pub fn remove_nodes_by<F: Fn(&Node) -> bool>(&mut self, filter: F) {
        let to_remove: Vec<Uuid> = self
            .nodes
            .iter()
            .filter(|(_, n)| filter(n))
            .map(|(&id, _)| id)
            .collect();

        for id in &to_remove {
            self.nodes.remove(id);
            self.responds_to.remove(id);
            self.invoked_by.remove(id);
            self.history.remove(id);
        }

        self.edges
            .retain(|e| !to_remove.contains(&e.from) && !to_remove.contains(&e.to));
    }

    /// Add a `ToolCall` node linked to its parent message via `Invoked` edge.
    /// Captures the Pending→Running transition as a version snapshot.
    pub fn add_tool_call(
        &mut self,
        id: Uuid,
        parent_message_id: Uuid,
        arguments: super::tool_types::ToolCallArguments,
        api_tool_use_id: Option<String>,
    ) -> Uuid {
        let node = Node::ToolCall {
            id,
            api_tool_use_id,
            arguments,
            status: ToolCallStatus::Pending,
            parent_message_id,
            created_at: Utc::now(),
            completed_at: None,
        };
        self.add_node(node);
        let _ = self.add_edge(id, parent_message_id, EdgeKind::Invoked);
        let _ = self.update_tool_call_status(id, ToolCallStatus::Running, None);
        id
    }

    /// Add a `ToolResult` node linked to its tool call via `Produced` edge.
    pub fn add_tool_result(
        &mut self,
        tool_call_id: Uuid,
        content: ToolResultContent,
        is_error: bool,
    ) -> Uuid {
        let result_id = Uuid::new_v4();
        let node = Node::ToolResult {
            id: result_id,
            tool_call_id,
            content,
            is_error,
            created_at: Utc::now(),
        };
        self.add_node(node);
        let _ = self.add_edge(result_id, tool_call_id, EdgeKind::Produced);
        result_id
    }
}
