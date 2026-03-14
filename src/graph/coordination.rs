//! Q/A queries and multi-agent coordination primitives.
//!
//! Provides readiness checks, atomic claiming, and question lifecycle queries.
//! These methods are used by event subscribers for routing and self-scheduling.

use super::event::GraphEvent;
use super::node::QuestionStatus;
use super::{ConversationGraph, EdgeKind, Node};
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
