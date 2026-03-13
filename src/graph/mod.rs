pub mod tool_types;

pub use tool_types::{parse_tool_arguments, ToolCallArguments, ToolCallStatus, ToolResultContent};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

// ── Enums ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    User,
    Assistant,
    System,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WorkItemStatus {
    Todo,
    Active,
    Done,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GitFileStatus {
    Tracked,
    Modified,
    Staged,
    Untracked,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BackgroundTaskKind {
    GitIndex,
    ContextSummarize,
    ToolDiscovery,
    ToolExtraction,
    AgentPhase,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Pending,
    Running,
    Completed,
    Failed,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EdgeKind {
    RespondsTo,
    SubtaskOf,
    RelevantTo,
    Tracks,
    Indexes,
    Provides,
    ThinkingOf,
    Invoked,
    Produced,
}

// ── Edge ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Edge {
    pub from: Uuid,
    pub to: Uuid,
    pub kind: EdgeKind,
}

// ── Node ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Node {
    Message {
        id: Uuid,
        role: Role,
        content: String,
        created_at: DateTime<Utc>,
        model: Option<String>,
        input_tokens: Option<u32>,
        output_tokens: Option<u32>,
    },
    SystemDirective {
        id: Uuid,
        content: String,
        created_at: DateTime<Utc>,
    },
    WorkItem {
        id: Uuid,
        title: String,
        status: WorkItemStatus,
        description: Option<String>,
        created_at: DateTime<Utc>,
    },
    GitFile {
        id: Uuid,
        path: String,
        status: GitFileStatus,
        updated_at: DateTime<Utc>,
    },
    Tool {
        id: Uuid,
        name: String,
        description: String,
        updated_at: DateTime<Utc>,
    },
    BackgroundTask {
        id: Uuid,
        kind: BackgroundTaskKind,
        status: TaskStatus,
        description: String,
        created_at: DateTime<Utc>,
        updated_at: DateTime<Utc>,
    },
    ThinkBlock {
        id: Uuid,
        content: String,
        parent_message_id: Uuid,
        created_at: DateTime<Utc>,
    },
    ToolCall {
        id: Uuid,
        /// The API-assigned `tool_use` ID (e.g. `toolu_xxx`), used to pair
        /// `tool_use`/`tool_result` blocks in the LLM context.
        api_tool_use_id: Option<String>,
        arguments: ToolCallArguments,
        status: ToolCallStatus,
        parent_message_id: Uuid,
        created_at: DateTime<Utc>,
        completed_at: Option<DateTime<Utc>>,
    },
    ToolResult {
        id: Uuid,
        tool_call_id: Uuid,
        content: ToolResultContent,
        is_error: bool,
        created_at: DateTime<Utc>,
    },
}

impl Node {
    pub fn id(&self) -> Uuid {
        match self {
            Node::Message { id, .. }
            | Node::SystemDirective { id, .. }
            | Node::WorkItem { id, .. }
            | Node::GitFile { id, .. }
            | Node::Tool { id, .. }
            | Node::BackgroundTask { id, .. }
            | Node::ThinkBlock { id, .. }
            | Node::ToolCall { id, .. }
            | Node::ToolResult { id, .. } => *id,
        }
    }

    pub fn content(&self) -> &str {
        match self {
            Node::Message { content, .. }
            | Node::SystemDirective { content, .. }
            | Node::ThinkBlock { content, .. } => content,
            Node::ToolResult { content, .. } => content.text_content(),
            Node::WorkItem { title, .. } => title,
            Node::GitFile { path, .. } => path,
            Node::Tool { name, .. } => name,
            Node::BackgroundTask { description, .. } => description,
            Node::ToolCall { arguments, .. } => arguments.tool_name(),
        }
    }

    pub fn input_tokens(&self) -> Option<u32> {
        match self {
            Node::Message { input_tokens, .. } => *input_tokens,
            _ => None,
        }
    }

    pub fn output_tokens(&self) -> Option<u32> {
        match self {
            Node::Message { output_tokens, .. } => *output_tokens,
            _ => None,
        }
    }
}

// ── ConversationGraph ────────────────────────────────────────────────

/// Serialization-friendly representation (matches V2 format).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(from = "ConversationGraphRaw", into = "ConversationGraphRaw")]
pub struct ConversationGraph {
    nodes: HashMap<Uuid, Node>,
    edges: Vec<Edge>,
    branches: HashMap<String, Uuid>,
    active_branch: String,
    /// Runtime index for fast ancestor walking. Not serialized.
    #[serde(skip)]
    responds_to: HashMap<Uuid, Uuid>,
    /// Runtime index: `ToolCall` id -> parent message id (from `Invoked` edges). Not serialized.
    #[serde(skip)]
    invoked_by: HashMap<Uuid, Uuid>,
}

/// Raw form for serde (no runtime indexes).
#[derive(Debug, Clone, Serialize, Deserialize)]
struct ConversationGraphRaw {
    nodes: HashMap<Uuid, Node>,
    edges: Vec<Edge>,
    branches: HashMap<String, Uuid>,
    active_branch: String,
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

    /// Update the status (and optionally `completed_at`) of a `ToolCall` node in place.
    pub fn update_tool_call_status(
        &mut self,
        id: Uuid,
        new_status: ToolCallStatus,
        completed_at: Option<DateTime<Utc>>,
    ) -> anyhow::Result<()> {
        let node = self
            .nodes
            .get_mut(&id)
            .ok_or_else(|| anyhow::anyhow!("Node {id} not found"))?;
        match node {
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
        }
    }

    /// Update the status, description, and `updated_at` of a `BackgroundTask` in place.
    /// Preserves `created_at` (unlike `upsert_node` which replaces the whole node).
    pub fn update_background_task_status(
        &mut self,
        id: Uuid,
        new_status: TaskStatus,
        new_description: String,
    ) -> anyhow::Result<()> {
        let node = self
            .nodes
            .get_mut(&id)
            .ok_or_else(|| anyhow::anyhow!("Node {id} not found"))?;
        match node {
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
        }
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

    /// Set the `input_tokens` field on a `Message` node.
    pub fn set_input_tokens(&mut self, node_id: Uuid, tokens: u32) {
        if let Some(Node::Message { input_tokens, .. }) = self.nodes.get_mut(&node_id) {
            *input_tokens = Some(tokens);
        }
    }

    /// Remove all nodes (and their edges) matching a predicate.
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
        }

        self.edges
            .retain(|e| !to_remove.contains(&e.from) && !to_remove.contains(&e.to));
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
#[path = "tool_types_tests.rs"]
mod tool_types_tests;

#[cfg(test)]
#[path = "tool_args_tests.rs"]
mod tool_args_tests;
