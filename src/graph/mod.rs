pub mod history;
mod mutation;
pub mod node;
pub mod tool_types;

pub use history::NodeSnapshot;
pub use node::{
    BackgroundTaskKind, Edge, EdgeKind, GitFileStatus, Node, Role, StopReason, TaskStatus,
    WorkItemStatus,
};
pub use tool_types::{parse_tool_arguments, ToolCallArguments, ToolResultContent};

use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

// ── ConversationGraph ────────────────────────────────────────────────

/// The conversation graph — single source of truth for all conversation state.
/// Mutation methods capture version history automatically via `mutate_node`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(from = "ConversationGraphRaw", into = "ConversationGraphRaw")]
pub struct ConversationGraph {
    pub(super) nodes: HashMap<Uuid, Node>,
    pub(super) edges: Vec<Edge>,
    branches: HashMap<String, Uuid>,
    active_branch: String,
    /// Version history: previous states of mutated nodes (oldest first per node).
    pub(super) history: HashMap<Uuid, Vec<NodeSnapshot>>,
    /// Runtime index for fast ancestor walking. Not serialized.
    #[serde(skip)]
    pub(super) responds_to: HashMap<Uuid, Uuid>,
    /// Runtime index: `ToolCall` id -> parent message id (from `Invoked` edges). Not serialized.
    #[serde(skip)]
    pub(super) invoked_by: HashMap<Uuid, Uuid>,
}

/// Raw form for serde (no runtime indexes).
#[derive(Debug, Clone, Serialize, Deserialize)]
struct ConversationGraphRaw {
    nodes: HashMap<Uuid, Node>,
    edges: Vec<Edge>,
    branches: HashMap<String, Uuid>,
    active_branch: String,
    #[serde(default)]
    history: HashMap<Uuid, Vec<NodeSnapshot>>,
}

impl From<ConversationGraphRaw> for ConversationGraph {
    fn from(raw: ConversationGraphRaw) -> Self {
        let responds_to = raw
            .edges
            .iter()
            .filter(|e| e.kind == EdgeKind::RespondsTo)
            .map(|e| (e.from, e.to))
            .collect();
        let invoked_by = raw
            .edges
            .iter()
            .filter(|e| e.kind == EdgeKind::Invoked)
            .map(|e| (e.from, e.to))
            .collect();
        Self {
            nodes: raw.nodes,
            edges: raw.edges,
            branches: raw.branches,
            active_branch: raw.active_branch,
            history: raw.history,
            responds_to,
            invoked_by,
        }
    }
}

impl From<ConversationGraph> for ConversationGraphRaw {
    fn from(g: ConversationGraph) -> Self {
        Self {
            nodes: g.nodes,
            edges: g.edges,
            branches: g.branches,
            active_branch: g.active_branch,
            history: g.history,
        }
    }
}

impl ConversationGraph {
    pub fn new(system_prompt: &str) -> Self {
        let id = Uuid::new_v4();
        let root = Node::SystemDirective {
            id,
            content: system_prompt.to_string(),
            created_at: Utc::now(),
        };
        let mut nodes = HashMap::new();
        nodes.insert(id, root);

        let mut branches = HashMap::new();
        branches.insert("main".to_string(), id);

        Self {
            nodes,
            edges: Vec::new(),
            branches,
            active_branch: "main".to_string(),
            history: HashMap::new(),
            responds_to: HashMap::new(),
            invoked_by: HashMap::new(),
        }
    }

    /// Add a message node as a child of `parent_id` via a `RespondsTo` edge.
    /// Updates the active branch leaf pointer.
    pub fn add_message(&mut self, parent_id: Uuid, node: Node) -> anyhow::Result<Uuid> {
        if !self.nodes.contains_key(&parent_id) {
            anyhow::bail!("Parent node {parent_id} does not exist");
        }
        let id = node.id();
        self.nodes.insert(id, node);
        self.edges.push(Edge {
            from: id,
            to: parent_id,
            kind: EdgeKind::RespondsTo,
        });
        self.responds_to.insert(id, parent_id);
        self.branches.insert(self.active_branch.clone(), id);
        Ok(id)
    }

    /// Insert a node without any edges.
    pub fn add_node(&mut self, node: Node) -> Uuid {
        let id = node.id();
        self.nodes.insert(id, node);
        id
    }

    /// Add a typed edge between two existing nodes.
    pub fn add_edge(&mut self, from: Uuid, to: Uuid, kind: EdgeKind) -> anyhow::Result<()> {
        if !self.nodes.contains_key(&from) {
            anyhow::bail!("Node {from} does not exist");
        }
        if !self.nodes.contains_key(&to) {
            anyhow::bail!("Node {to} does not exist");
        }
        match kind {
            EdgeKind::RespondsTo => {
                self.responds_to.insert(from, to);
            }
            EdgeKind::Invoked => {
                self.invoked_by.insert(from, to);
            }
            _ => {}
        }
        self.edges.push(Edge { from, to, kind });
        Ok(())
    }

    /// Walk from the branch leaf to the root via `RespondsTo` edges, return chronological order.
    pub fn get_branch_history(&self, branch_name: &str) -> anyhow::Result<Vec<&Node>> {
        let leaf_id = self
            .branches
            .get(branch_name)
            .ok_or_else(|| anyhow::anyhow!("Branch '{branch_name}' does not exist"))?;

        let mut path = Vec::new();
        let mut visited = HashSet::new();
        let mut current = *leaf_id;

        loop {
            if !visited.insert(current) {
                anyhow::bail!("Cycle detected in graph at node {current}");
            }
            let node = self
                .nodes
                .get(&current)
                .ok_or_else(|| anyhow::anyhow!("Node {current} not found"))?;
            path.push(node);

            match self.responds_to.get(&current) {
                Some(&parent) => current = parent,
                None => break,
            }
        }

        path.reverse();
        Ok(path)
    }

    pub fn active_branch(&self) -> &str {
        &self.active_branch
    }

    pub fn branch_leaf(&self, branch_name: &str) -> Option<Uuid> {
        self.branches.get(branch_name).copied()
    }

    /// Get the leaf node of the active branch, or error.
    pub fn active_leaf(&self) -> anyhow::Result<Uuid> {
        self.branch_leaf(&self.active_branch)
            .ok_or_else(|| anyhow::anyhow!("No leaf node for active branch"))
    }

    pub fn branch_names(&self) -> Vec<&str> {
        self.branches.keys().map(String::as_str).collect()
    }

    /// Return all nodes matching a predicate.
    pub fn nodes_by<F: Fn(&Node) -> bool>(&self, filter: F) -> Vec<&Node> {
        self.nodes.values().filter(|n| filter(n)).collect()
    }

    /// Find all nodes connected to `target` via edges of the given kind.
    /// Returns node ids where an edge `(source) --[kind]--> (target)` exists.
    pub fn sources_by_edge(&self, target: Uuid, kind: EdgeKind) -> Vec<Uuid> {
        self.edges
            .iter()
            .filter(|e| e.to == target && e.kind == kind)
            .map(|e| e.from)
            .collect()
    }

    /// Look up a node by id.
    pub fn node(&self, id: Uuid) -> Option<&Node> {
        self.nodes.get(&id)
    }

    /// Check if a node has an associated `ThinkBlock` linked via `ThinkingOf`.
    pub fn has_think_block(&self, node_id: Uuid) -> bool {
        self.edges
            .iter()
            .any(|e| e.to == node_id && e.kind == EdgeKind::ThinkingOf)
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
#[path = "mutation_tests.rs"]
mod mutation_tests;

#[cfg(test)]
#[path = "history_tests.rs"]
mod history_tests;

#[cfg(test)]
#[path = "tool_types_tests.rs"]
mod tool_types_tests;

#[cfg(test)]
#[path = "tool_args_tests.rs"]
mod tool_args_tests;
