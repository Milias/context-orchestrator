use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    User,
    Assistant,
    System,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Node {
    Message {
        id: Uuid,
        role: Role,
        content: String,
        created_at: DateTime<Utc>,
        model: Option<String>,
        token_count: Option<u32>,
    },
    SystemDirective {
        id: Uuid,
        content: String,
        created_at: DateTime<Utc>,
    },
}

#[allow(dead_code)]
impl Node {
    pub fn id(&self) -> Uuid {
        match self {
            Node::Message { id, .. } => *id,
            Node::SystemDirective { id, .. } => *id,
        }
    }

    pub fn content(&self) -> &str {
        match self {
            Node::Message { content, .. } => content,
            Node::SystemDirective { content, .. } => content,
        }
    }

    pub fn created_at(&self) -> DateTime<Utc> {
        match self {
            Node::Message { created_at, .. } => *created_at,
            Node::SystemDirective { created_at, .. } => *created_at,
        }
    }

    pub fn role(&self) -> Option<&Role> {
        match self {
            Node::Message { role, .. } => Some(role),
            Node::SystemDirective { .. } => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversationGraph {
    nodes: HashMap<Uuid, Node>,
    /// child_id -> parent_id (responds_to)
    edges: HashMap<Uuid, Uuid>,
    /// branch_name -> leaf_node_id
    branches: HashMap<String, Uuid>,
    /// Currently active branch name
    active_branch: String,
}

#[allow(dead_code)]
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
            edges: HashMap::new(),
            branches,
            active_branch: "main".to_string(),
        }
    }

    pub fn root_id(&self) -> Option<Uuid> {
        self.nodes
            .keys()
            .find(|id| !self.edges.contains_key(id))
            .copied()
    }

    pub fn add_message(&mut self, parent_id: Uuid, node: Node) -> anyhow::Result<Uuid> {
        if !self.nodes.contains_key(&parent_id) {
            anyhow::bail!("Parent node {} does not exist", parent_id);
        }
        let id = node.id();
        self.nodes.insert(id, node);
        self.edges.insert(id, parent_id);
        self.branches.insert(self.active_branch.clone(), id);
        Ok(id)
    }

    pub fn get_branch_history(&self, branch_name: &str) -> anyhow::Result<Vec<&Node>> {
        let leaf_id = self
            .branches
            .get(branch_name)
            .ok_or_else(|| anyhow::anyhow!("Branch '{}' does not exist", branch_name))?;

        let mut path = Vec::new();
        let mut visited = HashSet::new();
        let mut current = *leaf_id;

        loop {
            if !visited.insert(current) {
                anyhow::bail!("Cycle detected in graph at node {}", current);
            }
            let node = self
                .nodes
                .get(&current)
                .ok_or_else(|| anyhow::anyhow!("Node {} not found", current))?;
            path.push(node);

            match self.edges.get(&current) {
                Some(&parent) => current = parent,
                None => break, // reached root
            }
        }

        path.reverse();
        Ok(path)
    }

    pub fn create_branch(&mut self, name: &str, fork_point_id: Uuid) -> anyhow::Result<()> {
        if self.branches.contains_key(name) {
            anyhow::bail!("Branch '{}' already exists", name);
        }
        if !self.nodes.contains_key(&fork_point_id) {
            anyhow::bail!("Fork point node {} does not exist", fork_point_id);
        }
        self.branches.insert(name.to_string(), fork_point_id);
        Ok(())
    }

    pub fn get_children(&self, node_id: Uuid) -> Vec<Uuid> {
        self.edges
            .iter()
            .filter(|(_, &parent)| parent == node_id)
            .map(|(&child, _)| child)
            .collect()
    }

    pub fn active_branch(&self) -> &str {
        &self.active_branch
    }

    pub fn switch_branch(&mut self, branch_name: &str) -> anyhow::Result<()> {
        if !self.branches.contains_key(branch_name) {
            anyhow::bail!("Branch '{}' does not exist", branch_name);
        }
        self.active_branch = branch_name.to_string();
        Ok(())
    }

    pub fn branch_names(&self) -> Vec<&String> {
        let mut names: Vec<_> = self.branches.keys().collect();
        names.sort();
        names
    }

    pub fn branch_leaf(&self, branch_name: &str) -> Option<Uuid> {
        self.branches.get(branch_name).copied()
    }

    pub fn get_node(&self, id: Uuid) -> Option<&Node> {
        self.nodes.get(&id)
    }

    pub fn parent_of(&self, id: Uuid) -> Option<Uuid> {
        self.edges.get(&id).copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_creates_root_and_main_branch() {
        let graph = ConversationGraph::new("You are helpful.");
        assert_eq!(graph.active_branch(), "main");
        assert!(graph.root_id().is_some());

        let root = graph.get_node(graph.root_id().unwrap()).unwrap();
        assert_eq!(root.content(), "You are helpful.");
        assert!(root.role().is_none()); // SystemDirective has no role
    }

    #[test]
    fn test_add_message_and_history() {
        let mut graph = ConversationGraph::new("System prompt");
        let root_id = graph.root_id().unwrap();

        let user_msg = Node::Message {
            id: Uuid::new_v4(),
            role: Role::User,
            content: "Hello".to_string(),
            created_at: Utc::now(),
            model: None,
            token_count: None,
        };
        let user_id = graph.add_message(root_id, user_msg).unwrap();

        let asst_msg = Node::Message {
            id: Uuid::new_v4(),
            role: Role::Assistant,
            content: "Hi there".to_string(),
            created_at: Utc::now(),
            model: Some("claude".to_string()),
            token_count: Some(10),
        };
        let _asst_id = graph.add_message(user_id, asst_msg).unwrap();

        let history = graph.get_branch_history("main").unwrap();
        assert_eq!(history.len(), 3);
        assert_eq!(history[0].content(), "System prompt");
        assert_eq!(history[1].content(), "Hello");
        assert_eq!(history[2].content(), "Hi there");
    }

    #[test]
    fn test_branching() {
        let mut graph = ConversationGraph::new("System");
        let root_id = graph.root_id().unwrap();

        // Add user + assistant on main
        let user1 = Node::Message {
            id: Uuid::new_v4(),
            role: Role::User,
            content: "Q1".to_string(),
            created_at: Utc::now(),
            model: None,
            token_count: None,
        };
        let user1_id = graph.add_message(root_id, user1).unwrap();

        let asst1 = Node::Message {
            id: Uuid::new_v4(),
            role: Role::Assistant,
            content: "A1".to_string(),
            created_at: Utc::now(),
            model: None,
            token_count: None,
        };
        let asst1_id = graph.add_message(user1_id, asst1).unwrap();

        // Branch from asst1
        graph.create_branch("explore", asst1_id).unwrap();
        graph.switch_branch("explore").unwrap();

        // Add message on explore branch
        let user2 = Node::Message {
            id: Uuid::new_v4(),
            role: Role::User,
            content: "Q2-explore".to_string(),
            created_at: Utc::now(),
            model: None,
            token_count: None,
        };
        graph.add_message(asst1_id, user2).unwrap();

        // Add another message on main
        graph.switch_branch("main").unwrap();
        let user3 = Node::Message {
            id: Uuid::new_v4(),
            role: Role::User,
            content: "Q2-main".to_string(),
            created_at: Utc::now(),
            model: None,
            token_count: None,
        };
        graph.add_message(asst1_id, user3).unwrap();

        // Verify different histories
        let main_history = graph.get_branch_history("main").unwrap();
        let explore_history = graph.get_branch_history("explore").unwrap();

        assert_eq!(main_history.len(), 4); // system, Q1, A1, Q2-main
        assert_eq!(explore_history.len(), 4); // system, Q1, A1, Q2-explore
        assert_eq!(main_history[3].content(), "Q2-main");
        assert_eq!(explore_history[3].content(), "Q2-explore");
    }

    #[test]
    fn test_get_children() {
        let mut graph = ConversationGraph::new("System");
        let root_id = graph.root_id().unwrap();

        let child1 = Node::Message {
            id: Uuid::new_v4(),
            role: Role::User,
            content: "C1".to_string(),
            created_at: Utc::now(),
            model: None,
            token_count: None,
        };
        let c1_id = graph.add_message(root_id, child1).unwrap();

        // Create a second child of root via a branch
        graph.create_branch("b2", c1_id).unwrap();
        graph.switch_branch("b2").unwrap();
        // Actually we need a child of root, not c1. Let's create a branch from root.
        graph.create_branch("b3", root_id).unwrap();
        graph.switch_branch("b3").unwrap();
        let child2 = Node::Message {
            id: Uuid::new_v4(),
            role: Role::User,
            content: "C2".to_string(),
            created_at: Utc::now(),
            model: None,
            token_count: None,
        };
        graph.add_message(root_id, child2).unwrap();

        let children = graph.get_children(root_id);
        assert_eq!(children.len(), 2);
    }

    #[test]
    fn test_create_branch_duplicate_name_errors() {
        let graph_orig = ConversationGraph::new("System");
        let mut graph = graph_orig;
        let root_id = graph.root_id().unwrap();
        assert!(graph.create_branch("main", root_id).is_err());
    }

    #[test]
    fn test_create_branch_invalid_fork_point_errors() {
        let mut graph = ConversationGraph::new("System");
        let fake_id = Uuid::new_v4();
        assert!(graph.create_branch("new", fake_id).is_err());
    }

    #[test]
    fn test_switch_branch_nonexistent_errors() {
        let mut graph = ConversationGraph::new("System");
        assert!(graph.switch_branch("nonexistent").is_err());
    }

    #[test]
    fn test_serde_roundtrip() {
        let mut graph = ConversationGraph::new("System prompt");
        let root_id = graph.root_id().unwrap();

        let msg = Node::Message {
            id: Uuid::new_v4(),
            role: Role::User,
            content: "Hello".to_string(),
            created_at: Utc::now(),
            model: None,
            token_count: None,
        };
        graph.add_message(root_id, msg).unwrap();

        let json = serde_json::to_string_pretty(&graph).unwrap();
        let restored: ConversationGraph = serde_json::from_str(&json).unwrap();

        assert_eq!(restored.active_branch(), graph.active_branch());
        assert_eq!(restored.branch_names().len(), graph.branch_names().len());

        let orig_history = graph.get_branch_history("main").unwrap();
        let rest_history = restored.get_branch_history("main").unwrap();
        assert_eq!(orig_history.len(), rest_history.len());
        for (a, b) in orig_history.iter().zip(rest_history.iter()) {
            assert_eq!(a.id(), b.id());
            assert_eq!(a.content(), b.content());
        }
    }
}
