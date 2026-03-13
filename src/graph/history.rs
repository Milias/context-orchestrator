use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::node::Node;
use super::ConversationGraph;
use uuid::Uuid;

/// A timestamped snapshot of a node's state captured before a mutation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeSnapshot {
    /// The complete node state at the time of capture (before mutation).
    pub node: Node,
    /// When this snapshot was taken (just before the mutation applied).
    pub captured_at: DateTime<Utc>,
}

impl ConversationGraph {
    /// Returns the version history for a node (oldest first), or empty slice if none.
    pub fn node_history(&self, id: Uuid) -> &[NodeSnapshot] {
        self.history.get(&id).map_or(&[], Vec::as_slice)
    }
}
