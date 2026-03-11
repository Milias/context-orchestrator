use crate::graph::{ConversationGraph, Edge, EdgeKind, Node, Role};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use uuid::Uuid;

/// Current graph format version.
pub const CURRENT_VERSION: u32 = 2;

/// Intermediate struct matching `ConversationGraphRaw` fields without the version envelope.
#[derive(Serialize, Deserialize)]
struct GraphRaw {
    nodes: HashMap<Uuid, Node>,
    edges: Vec<Edge>,
    branches: HashMap<String, Uuid>,
    active_branch: String,
}

// ── V1 types (original format, no version field) ─────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct V1Graph {
    pub nodes: HashMap<Uuid, V1Node>,
    pub edges: HashMap<Uuid, Uuid>,
    pub branches: HashMap<String, Uuid>,
    pub active_branch: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum V1Node {
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

// ── V2 types (typed edges, new node types) ───────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct V2Graph {
    pub nodes: HashMap<Uuid, Node>,
    pub edges: Vec<Edge>,
    pub branches: HashMap<String, Uuid>,
    pub active_branch: String,
}

// ── Tagged union for versioned graphs ────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "version")]
pub enum VersionedGraph {
    #[serde(rename = "2")]
    V2(V2Graph),
}

/// Detect the version of a serialized graph.
/// Tries to deserialize as `VersionedGraph` (tagged union). If that fails
/// (V1 files have no `version` field), returns 1.
fn detect_version(data: &str) -> u32 {
    if serde_json::from_str::<VersionedGraph>(data).is_ok() {
        return 2;
    }
    1
}

// ── Migration functions ──────────────────────────────────────────────

fn v1_node_to_node(v1: V1Node) -> Node {
    match v1 {
        V1Node::Message {
            id,
            role,
            content,
            created_at,
            model,
            input_tokens,
            output_tokens,
        } => Node::Message {
            id,
            role,
            content,
            created_at,
            model,
            input_tokens,
            output_tokens,
        },
        V1Node::SystemDirective {
            id,
            content,
            created_at,
        } => Node::SystemDirective {
            id,
            content,
            created_at,
        },
    }
}

fn migrate_v1_to_v2(v1: V1Graph) -> V2Graph {
    let nodes = v1
        .nodes
        .into_iter()
        .map(|(id, n)| (id, v1_node_to_node(n)))
        .collect();
    let edges = v1
        .edges
        .into_iter()
        .map(|(child, parent)| Edge {
            from: child,
            to: parent,
            kind: EdgeKind::RespondsTo,
        })
        .collect();
    V2Graph {
        nodes,
        edges,
        branches: v1.branches,
        active_branch: v1.active_branch,
    }
}

// ── Public API ───────────────────────────────────────────────────────

/// Back up a graph file before migration.
fn backup_graph(graph_path: &Path, version: u32) -> anyhow::Result<()> {
    let backup_name = format!(
        "graph.v{version}.json.bak",
    );
    let backup_path = graph_path.with_file_name(backup_name);
    std::fs::copy(graph_path, &backup_path)?;
    Ok(())
}

/// Load a graph from JSON, migrating from older versions if needed.
/// If migration occurs, backs up the original file and writes the migrated version.
pub fn load_and_migrate(graph_path: &Path) -> anyhow::Result<ConversationGraph> {
    let data = std::fs::read_to_string(graph_path)?;
    let version = detect_version(&data);

    if version >= CURRENT_VERSION {
        let versioned: VersionedGraph = serde_json::from_str(&data)?;
        let VersionedGraph::V2(v2) = versioned;
        return v2_to_conversation_graph(v2);
    }

    // Migration needed
    backup_graph(graph_path, version)?;

    let v2 = match version {
        1 => {
            let v1: V1Graph = serde_json::from_str(&data)?;
            migrate_v1_to_v2(v1)
        }
        other => anyhow::bail!("Unknown graph version: {other}"),
    };

    // Write the migrated graph wrapped in the versioned envelope
    let migrated_json = serde_json::to_string_pretty(&VersionedGraph::V2(v2.clone()))?;
    let tmp_path = graph_path.with_extension("json.tmp");
    std::fs::write(&tmp_path, &migrated_json)?;
    std::fs::rename(&tmp_path, graph_path)?;

    v2_to_conversation_graph(v2)
}

/// Convert a `V2Graph` to the live `ConversationGraph`.
fn v2_to_conversation_graph(v2: V2Graph) -> anyhow::Result<ConversationGraph> {
    let raw = GraphRaw {
        nodes: v2.nodes,
        edges: v2.edges,
        branches: v2.branches,
        active_branch: v2.active_branch,
    };

    let json = serde_json::to_string(&raw)?;
    let graph: ConversationGraph = serde_json::from_str(&json)?;
    Ok(graph)
}

/// Wrap a `ConversationGraph` in the V2 envelope for saving.
pub fn to_versioned_json(graph: &ConversationGraph) -> anyhow::Result<String> {
    let raw_json = serde_json::to_string(graph)?;
    let raw: GraphRaw = serde_json::from_str(&raw_json)?;
    let v2 = V2Graph {
        nodes: raw.nodes,
        edges: raw.edges,
        branches: raw.branches,
        active_branch: raw.active_branch,
    };
    let versioned = VersionedGraph::V2(v2);

    Ok(serde_json::to_string_pretty(&versioned)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::ConversationGraph;

    #[test]
    fn test_detect_version_v1_no_field() {
        let v1_json = r#"{"nodes":{},"edges":{},"branches":{"main":"00000000-0000-0000-0000-000000000000"},"active_branch":"main"}"#;
        assert_eq!(detect_version(v1_json), 1);
    }

    #[test]
    fn test_detect_version_v2() {
        let v2_json = r#"{"version":"2","nodes":{},"edges":[],"branches":{"main":"00000000-0000-0000-0000-000000000000"},"active_branch":"main"}"#;
        assert_eq!(detect_version(v2_json), 2);
    }

    #[test]
    fn test_v1_migration_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        let graph_path = tmp.path().join("graph.json");

        // Create a V1-format graph (the old format)
        let graph = ConversationGraph::new("Test prompt");
        let root_id = graph.branch_leaf("main").unwrap();

        // Manually build V1 JSON (old format with HashMap edges)
        let v1 = V1Graph {
            nodes: {
                let mut m = HashMap::new();
                m.insert(
                    root_id,
                    V1Node::SystemDirective {
                        id: root_id,
                        content: "Test prompt".to_string(),
                        created_at: Utc::now(),
                    },
                );
                m
            },
            edges: HashMap::new(),
            branches: {
                let mut b = HashMap::new();
                b.insert("main".to_string(), root_id);
                b
            },
            active_branch: "main".to_string(),
        };

        // Write V1 format (no version field)
        std::fs::write(&graph_path, serde_json::to_string_pretty(&v1).unwrap()).unwrap();

        // Load and migrate
        let loaded = load_and_migrate(&graph_path).unwrap();
        assert_eq!(loaded.active_branch(), "main");
        let history = loaded.get_branch_history("main").unwrap();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].content(), "Test prompt");

        // Check backup was created
        let backup_path = tmp.path().join("graph.v1.json.bak");
        assert!(backup_path.exists());

        // Check migrated file has version field
        let migrated_data = std::fs::read_to_string(&graph_path).unwrap();
        assert_eq!(detect_version(&migrated_data), 2);
    }

    #[test]
    fn test_v2_load_no_migration() {
        let tmp = tempfile::tempdir().unwrap();
        let graph_path = tmp.path().join("graph.json");

        // Create and save a current-format graph
        let graph = ConversationGraph::new("V2 prompt");
        let json = to_versioned_json(&graph).unwrap();
        std::fs::write(&graph_path, &json).unwrap();

        // Load -- should not create a backup
        let loaded = load_and_migrate(&graph_path).unwrap();
        assert_eq!(loaded.active_branch(), "main");
        let history = loaded.get_branch_history("main").unwrap();
        assert_eq!(history[0].content(), "V2 prompt");

        // No backup should exist
        let backup_path = tmp.path().join("graph.v2.json.bak");
        assert!(!backup_path.exists());
    }

    #[test]
    fn test_to_versioned_json_includes_version() {
        let graph = ConversationGraph::new("Test");
        let json = to_versioned_json(&graph).unwrap();
        assert_eq!(detect_version(&json), 2);
    }
}
