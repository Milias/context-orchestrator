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
        input_tokens: Option<u32>,
        output_tokens: Option<u32>,
    },
    SystemDirective {
        id: Uuid,
        content: String,
        created_at: DateTime<Utc>,
    },
}

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

    pub fn active_branch(&self) -> &str {
        &self.active_branch
    }

    pub fn branch_leaf(&self, branch_name: &str) -> Option<Uuid> {
        self.branches.get(branch_name).copied()
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
}
