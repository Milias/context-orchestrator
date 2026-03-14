mod coordination;
pub mod event;
pub mod history;
mod mutation;
pub mod node;
pub mod tool;

/// Convenience alias: `graph::tool_types` → `graph::tool::types`.
/// Preserves existing import paths across the codebase.
pub use tool::types as tool_types;

pub use history::NodeSnapshot;
pub use node::{
    BackgroundTaskKind, Edge, EdgeKind, GitFileStatus, Node, Role, StopReason, TaskStatus,
    WorkItemKind, WorkItemStatus,
};
pub use tool::result::ToolResultContent;
pub use tool::types::{parse_tool_arguments, ToolCallArguments};

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
    /// Runtime index: child → parent for fast ancestor walking (from `RespondsTo` edges). Not serialized.
    #[serde(skip)]
    pub(super) responds_to: HashMap<Uuid, Uuid>,
    /// Runtime index: parent → children for fast forward traversal (inverse of `responds_to`). Not serialized.
    #[serde(skip)]
    pub(super) reply_children: HashMap<Uuid, Vec<Uuid>>,
    /// Runtime index: `ToolCall` id → parent message id (from `Invoked` edges). Not serialized.
    #[serde(skip)]
    pub(super) invoked_by: HashMap<Uuid, Uuid>,
    /// Event broadcast bus. Runtime-only (not serialized). `None` during tests
    /// and deserialization; initialized at app startup via `set_event_bus`.
    #[serde(skip)]
    event_bus: Option<event::EventBus>,
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
        let responds_to: HashMap<Uuid, Uuid> = raw
            .edges
            .iter()
            .filter(|e| e.kind == EdgeKind::RespondsTo)
            .map(|e| (e.from, e.to))
            .collect();
        // Build the forward index (parent → children) from the backward index.
        let mut reply_children: HashMap<Uuid, Vec<Uuid>> = HashMap::new();
        for (&child, &parent) in &responds_to {
            reply_children.entry(parent).or_default().push(child);
        }
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
            reply_children,
            invoked_by,
            event_bus: None,
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
            reply_children: HashMap::new(),
            invoked_by: HashMap::new(),
            event_bus: None,
        }
    }

    /// Initialize the event bus. Call once at app startup after loading/creating
    /// the graph. Returns a receiver for subscribing to events.
    pub fn init_event_bus(&mut self) -> tokio::sync::broadcast::Receiver<event::GraphEvent> {
        let bus = event::EventBus::new();
        let rx = bus.subscribe();
        self.event_bus = Some(bus);
        rx
    }

    /// Subscribe to the event bus. Returns `None` if the bus is not initialized.
    /// Used by event dispatch to receive graph mutation notifications.
    pub fn subscribe_events(&self) -> Option<tokio::sync::broadcast::Receiver<event::GraphEvent>> {
        self.event_bus.as_ref().map(event::EventBus::subscribe)
    }

    /// Emit a graph event to all subscribers. No-op if the bus is not initialized.
    pub(crate) fn emit(&self, event: event::GraphEvent) {
        if let Some(bus) = &self.event_bus {
            bus.emit(event);
        }
    }

    /// Add a reply node linked to `parent_id` via a `RespondsTo` edge. Does NOT
    /// update any branch leaf pointer. Use for task agent messages and
    /// non-conversational replies that form their own `RespondsTo` chains.
    pub fn add_reply(&mut self, parent_id: Uuid, node: Node) -> anyhow::Result<Uuid> {
        if !self.nodes.contains_key(&parent_id) {
            anyhow::bail!("Parent node {parent_id} does not exist");
        }
        let id = node.id();
        let role = match &node {
            Node::Message { role, .. } => Some(*role),
            _ => None,
        };
        self.nodes.insert(id, node);
        self.edges.push(Edge {
            from: id,
            to: parent_id,
            kind: EdgeKind::RespondsTo,
        });
        self.responds_to.insert(id, parent_id);
        self.reply_children.entry(parent_id).or_default().push(id);
        if let Some(role) = role {
            self.emit(event::GraphEvent::MessageAdded { node_id: id, role });
        }
        Ok(id)
    }

    /// Add a message node as a child of `parent_id` via a `RespondsTo` edge.
    /// Updates the active branch leaf pointer. Use for the conversational agent
    /// whose messages advance the main conversation branch.
    pub fn add_message(&mut self, parent_id: Uuid, node: Node) -> anyhow::Result<Uuid> {
        let id = self.add_reply(parent_id, node)?;
        self.branches.insert(self.active_branch.clone(), id);
        Ok(id)
    }

    /// Walk forward from `root_id` through the `RespondsTo` chain to find the
    /// leaf node (the most recent reply). Returns `root_id` if no children exist.
    /// Uses the `reply_children` forward index for O(depth) traversal.
    pub fn find_chain_leaf(&self, root_id: Uuid) -> Uuid {
        let mut current = root_id;
        while let Some(children) = self.reply_children.get(&current) {
            if children.is_empty() {
                break;
            }
            // Follow the last child (most recently added reply in the chain).
            current = *children.last().expect("non-empty checked above");
        }
        current
    }

    /// Insert a node without any edges. Does NOT emit events — callers that
    /// create domain-significant nodes (`Question`, `WorkItem`) must emit the
    /// appropriate `GraphEvent` themselves after calling this.
    pub fn add_node(&mut self, node: Node) -> Uuid {
        let id = node.id();
        self.nodes.insert(id, node);
        id
    }

    /// Add a typed edge between two nodes. Both endpoints must exist in the graph,
    /// except for `ClaimedBy` edges where `to` is an agent UUID (not a graph node).
    pub fn add_edge(&mut self, from: Uuid, to: Uuid, kind: EdgeKind) -> anyhow::Result<()> {
        if !self.nodes.contains_key(&from) {
            anyhow::bail!("Node {from} does not exist");
        }
        // ClaimedBy targets an agent UUID, not a graph node.
        if kind != EdgeKind::ClaimedBy && !self.nodes.contains_key(&to) {
            anyhow::bail!("Node {to} does not exist");
        }
        match kind {
            EdgeKind::RespondsTo => {
                self.responds_to.insert(from, to);
                self.reply_children.entry(to).or_default().push(from);
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

    /// Look up a node by id (mutable). Use sparingly — prefer `mutate_node`
    /// in `mutation.rs` for changes that need version snapshots.
    pub fn node_mut(&mut self, id: Uuid) -> Option<&mut Node> {
        self.nodes.get_mut(&id)
    }

    /// Find all children of a node connected via `SubtaskOf` edges.
    /// Returns child node IDs (where `child --SubtaskOf--> parent`).
    pub fn children_of(&self, parent_id: Uuid) -> Vec<Uuid> {
        self.sources_by_edge(parent_id, EdgeKind::SubtaskOf)
    }

    /// Find the parent of a node via a `SubtaskOf` edge, if any.
    /// Returns the target of the first `SubtaskOf` edge from this node.
    pub fn parent_of(&self, child_id: Uuid) -> Option<Uuid> {
        self.edges
            .iter()
            .find(|e| e.from == child_id && e.kind == EdgeKind::SubtaskOf)
            .map(|e| e.to)
    }

    /// Find all plans that `plan_id` depends on (via `DependsOn` edges).
    /// Returns prerequisite plan IDs.
    pub fn dependencies_of(&self, plan_id: Uuid) -> Vec<Uuid> {
        self.edges
            .iter()
            .filter(|e| e.from == plan_id && e.kind == EdgeKind::DependsOn)
            .map(|e| e.to)
            .collect()
    }

    /// Check if there's a path from `start` to `target` following `DependsOn` edges.
    /// Used for cycle detection before adding a new dependency.
    pub fn has_dependency_path(&self, start: Uuid, target: Uuid) -> bool {
        let mut visited = std::collections::HashSet::new();
        let mut stack = vec![start];
        while let Some(current) = stack.pop() {
            if current == target {
                return true;
            }
            if !visited.insert(current) {
                continue;
            }
            for dep in self.dependencies_of(current) {
                stack.push(dep);
            }
        }
        false
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
#[path = "question_tests.rs"]
mod question_tests;

#[cfg(test)]
#[path = "event_tests.rs"]
mod event_tests;

#[cfg(test)]
#[path = "coordination_tests.rs"]
mod coordination_tests;
