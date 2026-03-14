use chrono::{DateTime, Utc};
use uuid::Uuid;

use super::history::NodeSnapshot;
use super::node::Node;
use super::tool_types::{ToolCallStatus, ToolResultContent};
use super::{ConversationGraph, EdgeKind, TaskStatus, WorkItemStatus};

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
        let is_failed = new_status == ToolCallStatus::Failed;
        let is_terminal = matches!(
            new_status,
            ToolCallStatus::Completed | ToolCallStatus::Failed
        );
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
        })?;
        if is_terminal {
            self.emit(super::event::GraphEvent::ToolCallCompleted {
                node_id: id,
                is_error: is_failed,
            });
        }
        Ok(())
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

    /// Update the status of a `WorkItem` node and propagate upward.
    /// When all siblings of a parent are `Done`, the parent auto-transitions to `Done`.
    /// When any child becomes `Active` and parent is `Todo`, parent becomes `Active`.
    pub fn update_work_item_status(
        &mut self,
        id: Uuid,
        new_status: WorkItemStatus,
    ) -> anyhow::Result<()> {
        let status_for_event = new_status.clone();
        self.mutate_node(id, |node| match node {
            Node::WorkItem { status, .. } => {
                *status = new_status;
                Ok(())
            }
            _ => anyhow::bail!("Node {id} is not a WorkItem"),
        })?;
        self.emit(super::event::GraphEvent::WorkItemStatusChanged {
            node_id: id,
            new_status: status_for_event,
        });
        self.propagate_status(id);
        Ok(())
    }

    /// Walk `SubtaskOf` edges upward, auto-transitioning parent status:
    /// - If all children are `Done` → parent becomes `Done`
    /// - If any child is `Active` and parent is `Todo` → parent becomes `Active`
    ///
    /// Propagation only moves status forward (`Todo` → `Active` → `Done`).
    /// If a child reverts from `Done`, the parent is NOT automatically reverted.
    /// This is intentional — completed plans should not auto-reopen.
    fn propagate_status(&mut self, child_id: Uuid) {
        let Some(parent_id) = self.parent_of(child_id) else {
            return;
        };
        let siblings = self.children_of(parent_id);
        let all_done = siblings.iter().all(|&sib| {
            matches!(
                self.node(sib),
                Some(Node::WorkItem {
                    status: WorkItemStatus::Done,
                    ..
                })
            )
        });
        let any_active = siblings.iter().any(|&sib| {
            matches!(
                self.node(sib),
                Some(Node::WorkItem {
                    status: WorkItemStatus::Active,
                    ..
                })
            )
        });

        let parent_status = self.node(parent_id).and_then(|n| match n {
            Node::WorkItem { status, .. } => Some(status.clone()),
            _ => None,
        });

        let new_parent_status = if all_done {
            Some(WorkItemStatus::Done)
        } else if any_active && parent_status == Some(WorkItemStatus::Todo) {
            Some(WorkItemStatus::Active)
        } else {
            None
        };

        if let Some(new_status) = new_parent_status {
            let _ = self.mutate_node(parent_id, |node| {
                if let Node::WorkItem { status, .. } = node {
                    *status = new_status.clone();
                }
                Ok(())
            });
            // Continue propagating upward.
            self.propagate_status(parent_id);
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

    /// Transition a `Question` node's status, validating the state machine.
    /// Captures a version snapshot before the mutation.
    ///
    /// See `QuestionStatus` for valid transitions.
    pub fn update_question_status(
        &mut self,
        id: Uuid,
        new_status: super::node::QuestionStatus,
    ) -> anyhow::Result<()> {
        use super::node::QuestionStatus;
        self.mutate_node(id, |node| match node {
            Node::Question { status, .. } => {
                let valid = matches!(
                    (*status, new_status),
                    (
                        QuestionStatus::Pending,
                        QuestionStatus::Claimed | QuestionStatus::TimedOut
                    ) | (
                        QuestionStatus::Claimed,
                        QuestionStatus::Answered | QuestionStatus::PendingApproval
                    ) | (
                        QuestionStatus::PendingApproval,
                        QuestionStatus::Answered | QuestionStatus::Rejected
                    ) | (QuestionStatus::Rejected, QuestionStatus::Pending)
                );
                if !valid {
                    anyhow::bail!(
                        "Invalid question status transition: {status:?} → {new_status:?}"
                    );
                }
                *status = new_status;
                Ok(())
            }
            _ => anyhow::bail!("Node {id} is not a Question"),
        })?;
        self.emit(super::event::GraphEvent::QuestionStatusChanged {
            node_id: id,
            new_status,
        });
        Ok(())
    }

    /// Create an `Answer` node wired to its `Question` via an `Answers` edge.
    /// Transitions the question to `Answered` (or `PendingApproval` if `requires_approval`).
    pub fn add_answer(&mut self, question_id: Uuid, content: String) -> anyhow::Result<Uuid> {
        use super::node::QuestionStatus;
        let requires_approval = match self.node(question_id) {
            Some(Node::Question {
                requires_approval,
                status,
                ..
            }) => {
                // Only Claimed questions can receive answers.
                if *status != QuestionStatus::Claimed {
                    anyhow::bail!("Question {question_id} is not in Claimed state (is {status:?})");
                }
                *requires_approval
            }
            _ => anyhow::bail!("Node {question_id} is not a Question"),
        };

        let answer_id = Uuid::new_v4();
        let answer = Node::Answer {
            id: answer_id,
            content,
            question_id,
            created_at: Utc::now(),
        };
        self.add_node(answer);
        let _ = self.add_edge(answer_id, question_id, EdgeKind::Answers);

        let target_status = if requires_approval {
            QuestionStatus::PendingApproval
        } else {
            QuestionStatus::Answered
        };
        self.update_question_status(question_id, target_status)?;

        if target_status == super::node::QuestionStatus::Answered {
            self.emit(super::event::GraphEvent::QuestionAnswered {
                question_id,
                answer_id,
            });
        }

        Ok(answer_id)
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
