//! Q/A queries and multi-agent coordination primitives.
//!
//! Provides readiness checks, atomic claiming, and question lifecycle queries.
//! These methods are used by event subscribers for routing and self-scheduling.

use super::event::GraphEvent;
use super::node::QuestionStatus;
use super::{ConversationGraph, EdgeKind, Node, TaskStatus, WorkItemStatus};
use uuid::Uuid;

impl ConversationGraph {
    /// All `Question` nodes not yet resolved (status ≠ `Answered`, `TimedOut`).
    pub fn open_questions(&self) -> Vec<&Node> {
        self.nodes_by(|n| {
            matches!(
                n,
                Node::Question { status, .. }
                if *status != QuestionStatus::Answered && *status != QuestionStatus::TimedOut
            )
        })
    }

    /// All `Question` nodes with `Pending` status (available for routing/claiming).
    pub fn pending_questions(&self) -> Vec<&Node> {
        self.nodes_by(|n| {
            matches!(
                n,
                Node::Question {
                    status: QuestionStatus::Pending,
                    ..
                }
            )
        })
    }

    /// Whether a node counts as "resolved" for `DependsOn` purposes.
    /// A resolved node no longer blocks dependents.
    pub fn is_resolved(&self, node_id: Uuid) -> bool {
        matches!(
            self.node(node_id),
            Some(
                Node::WorkItem {
                    status: WorkItemStatus::Done,
                    ..
                } | Node::Question {
                    status: QuestionStatus::Answered,
                    ..
                } | Node::BackgroundTask {
                    status: TaskStatus::Completed,
                    ..
                }
            )
        )
    }

    /// All nodes whose `DependsOn` targets are all resolved and that have no
    /// `ClaimedBy` edge. These are ready for an agent to claim and process.
    ///
    /// Nodes with zero dependencies are excluded — they were never blocked and
    /// don't participate in the scheduling system. Only nodes that had blocking
    /// prerequisites (and those prerequisites are now resolved) appear here.
    pub fn ready_unclaimed_nodes(&self) -> Vec<Uuid> {
        self.nodes
            .keys()
            .copied()
            .filter(|&id| {
                let deps = self.dependencies_of(id);
                if deps.is_empty() {
                    return false;
                }
                let all_resolved = deps.iter().all(|&dep| self.is_resolved(dep));
                all_resolved && !self.is_claimed(id)
            })
            .collect()
    }

    /// Whether a node has a `ClaimedBy` edge (is currently assigned to an agent).
    pub fn is_claimed(&self, node_id: Uuid) -> bool {
        self.edges
            .iter()
            .any(|e| e.from == node_id && e.kind == EdgeKind::ClaimedBy)
    }

    /// Atomically claim a node for an agent by adding a `ClaimedBy` edge.
    /// Returns `false` if already claimed (prevents double-execution).
    /// Must be called under a write lock for atomicity.
    pub fn try_claim(&mut self, node_id: Uuid, agent_id: Uuid) -> bool {
        if self.is_claimed(node_id) {
            return false;
        }
        let _ = self.add_edge(node_id, agent_id, EdgeKind::ClaimedBy);
        self.emit(GraphEvent::NodeClaimed { node_id, agent_id });
        true
    }

    /// Release a `ClaimedBy` edge on a node (for cancellation or crash recovery).
    pub fn release_claim(&mut self, node_id: Uuid) {
        self.edges
            .retain(|e| !(e.from == node_id && e.kind == EdgeKind::ClaimedBy));
    }

    /// Release all `ClaimedBy` edges in the graph (startup crash recovery).
    pub fn release_all_claims(&mut self) {
        self.edges.retain(|e| e.kind != EdgeKind::ClaimedBy);
    }
}
