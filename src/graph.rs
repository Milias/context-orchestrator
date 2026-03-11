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
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Pending,
    Running,
    Completed,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EdgeKind {
    RespondsTo,
    SubtaskOf,
    RelevantTo,
    Tracks,
    Indexes,
    Provides,
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
}

impl Node {
    pub fn id(&self) -> Uuid {
        match self {
            Node::Message { id, .. }
            | Node::SystemDirective { id, .. }
            | Node::WorkItem { id, .. }
            | Node::GitFile { id, .. }
            | Node::Tool { id, .. }
            | Node::BackgroundTask { id, .. } => *id,
        }
    }

    pub fn content(&self) -> &str {
        match self {
            Node::Message { content, .. } | Node::SystemDirective { content, .. } => content,
            Node::WorkItem { title, .. } => title,
            Node::GitFile { path, .. } => path,
            Node::Tool { name, .. } => name,
            Node::BackgroundTask { description, .. } => description,
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
        Self {
            nodes: raw.nodes,
            edges: raw.edges,
            branches: raw.branches,
            active_branch: raw.active_branch,
            responds_to,
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
        if kind == EdgeKind::RespondsTo {
            self.responds_to.insert(from, to);
        }
        self.edges.push(Edge { from, to, kind });
        Ok(())
    }

    /// Insert or update a node. Creates if absent, replaces if present.
    pub fn upsert_node(&mut self, node: Node) {
        self.nodes.insert(node.id(), node);
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
        }

        self.edges
            .retain(|e| !to_remove.contains(&e.from) && !to_remove.contains(&e.to));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_creates_root_and_main_branch() {
        let graph = ConversationGraph::new("You are helpful.");
        assert_eq!(graph.active_branch(), "main");

        let history = graph.get_branch_history("main").unwrap();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].content(), "You are helpful.");
    }

    #[test]
    fn test_add_message_and_history() {
        let mut graph = ConversationGraph::new("System prompt");
        let root_id = graph.branch_leaf("main").unwrap();

        let user_msg = Node::Message {
            id: Uuid::new_v4(),
            role: Role::User,
            content: "Hello".to_string(),
            created_at: Utc::now(),
            model: None,
            input_tokens: None,
            output_tokens: None,
        };
        let user_id = graph.add_message(root_id, user_msg).unwrap();

        let asst_msg = Node::Message {
            id: Uuid::new_v4(),
            role: Role::Assistant,
            content: "Hi there".to_string(),
            created_at: Utc::now(),
            model: Some("claude".to_string()),
            input_tokens: Some(25),
            output_tokens: Some(10),
        };
        let _asst_id = graph.add_message(user_id, asst_msg).unwrap();

        let history = graph.get_branch_history("main").unwrap();
        assert_eq!(history.len(), 3);
        assert_eq!(history[0].content(), "System prompt");
        assert_eq!(history[1].content(), "Hello");
        assert_eq!(history[2].content(), "Hi there");
    }

    #[test]
    fn test_serde_roundtrip() {
        let mut graph = ConversationGraph::new("System prompt");
        let root_id = graph.branch_leaf("main").unwrap();

        let msg = Node::Message {
            id: Uuid::new_v4(),
            role: Role::User,
            content: "Hello".to_string(),
            created_at: Utc::now(),
            model: None,
            input_tokens: None,
            output_tokens: None,
        };
        graph.add_message(root_id, msg).unwrap();

        let json = serde_json::to_string_pretty(&graph).unwrap();
        let restored: ConversationGraph = serde_json::from_str(&json).unwrap();

        assert_eq!(restored.active_branch(), graph.active_branch());

        let orig_history = graph.get_branch_history("main").unwrap();
        let rest_history = restored.get_branch_history("main").unwrap();
        assert_eq!(orig_history.len(), rest_history.len());
        for (a, b) in orig_history.iter().zip(rest_history.iter()) {
            assert_eq!(a.id(), b.id());
            assert_eq!(a.content(), b.content());
        }
    }

    #[test]
    fn test_add_node_without_edges() {
        let mut graph = ConversationGraph::new("System prompt");
        let work_item = Node::WorkItem {
            id: Uuid::new_v4(),
            title: "Fix the bug".to_string(),
            status: WorkItemStatus::Todo,
            description: None,
            created_at: Utc::now(),
        };
        let wi_id = graph.add_node(work_item);
        let found = graph.nodes_by(|n| matches!(n, Node::WorkItem { .. }));
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].id(), wi_id);
    }

    #[test]
    fn test_typed_edges() {
        let mut graph = ConversationGraph::new("System prompt");
        let root_id = graph.branch_leaf("main").unwrap();

        let wi = Node::WorkItem {
            id: Uuid::new_v4(),
            title: "Task".to_string(),
            status: WorkItemStatus::Active,
            description: None,
            created_at: Utc::now(),
        };
        let wi_id = graph.add_node(wi);

        graph
            .add_edge(wi_id, root_id, EdgeKind::RelevantTo)
            .unwrap();
        assert_eq!(graph.edges.len(), 1);
        assert_eq!(graph.edges[0].kind, EdgeKind::RelevantTo);
    }

    #[test]
    fn test_upsert_node() {
        let mut graph = ConversationGraph::new("System prompt");
        let id = Uuid::new_v4();

        // Upsert creates when absent
        let task = Node::BackgroundTask {
            id,
            kind: BackgroundTaskKind::GitIndex,
            status: TaskStatus::Running,
            description: "Indexing...".to_string(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        graph.upsert_node(task);
        assert_eq!(graph.nodes_by(|n| matches!(n, Node::BackgroundTask { status: TaskStatus::Running, .. })).len(), 1);

        // Upsert replaces when present
        let updated = Node::BackgroundTask {
            id,
            kind: BackgroundTaskKind::GitIndex,
            status: TaskStatus::Completed,
            description: "Indexing...".to_string(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        graph.upsert_node(updated);
        assert_eq!(graph.nodes_by(|n| matches!(n, Node::BackgroundTask { status: TaskStatus::Completed, .. })).len(), 1);
        assert_eq!(graph.nodes_by(|n| matches!(n, Node::BackgroundTask { status: TaskStatus::Running, .. })).len(), 0);
    }

    #[test]
    fn test_remove_nodes_by() {
        let mut graph = ConversationGraph::new("System prompt");
        let root_id = graph.branch_leaf("main").unwrap();

        let gf1 = Node::GitFile {
            id: Uuid::new_v4(),
            path: "src/main.rs".to_string(),
            status: GitFileStatus::Tracked,
            updated_at: Utc::now(),
        };
        let gf1_id = graph.add_node(gf1);
        graph
            .add_edge(gf1_id, root_id, EdgeKind::Indexes)
            .unwrap();

        let gf2 = Node::GitFile {
            id: Uuid::new_v4(),
            path: "src/lib.rs".to_string(),
            status: GitFileStatus::Modified,
            updated_at: Utc::now(),
        };
        graph.add_node(gf2);

        assert_eq!(
            graph
                .nodes_by(|n| matches!(n, Node::GitFile { .. }))
                .len(),
            2
        );

        graph.remove_nodes_by(|n| matches!(n, Node::GitFile { .. }));

        assert_eq!(
            graph
                .nodes_by(|n| matches!(n, Node::GitFile { .. }))
                .len(),
            0
        );
        // Edge referencing removed node should also be gone
        assert!(graph.edges.is_empty());
    }

    #[test]
    fn test_branch_names() {
        let graph = ConversationGraph::new("System prompt");
        let names = graph.branch_names();
        assert_eq!(names.len(), 1);
        assert!(names.contains(&"main"));
    }
}
