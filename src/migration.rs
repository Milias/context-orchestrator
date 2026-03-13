use crate::graph::{ConversationGraph, Edge, EdgeKind, Node, NodeSnapshot, Role};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use uuid::Uuid;

/// Current graph format version.
pub const CURRENT_VERSION: u32 = 3;

/// Intermediate struct matching `ConversationGraphRaw` fields without the version envelope.
#[derive(Serialize, Deserialize)]
struct GraphRaw {
    nodes: HashMap<Uuid, Node>,
    edges: Vec<Edge>,
    branches: HashMap<String, Uuid>,
    active_branch: String,
    #[serde(default)]
    history: HashMap<Uuid, Vec<NodeSnapshot>>,
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

// ── V3 types (node versioning history, stop_reason on Message) ───────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct V3Graph {
    pub nodes: HashMap<Uuid, Node>,
    pub edges: Vec<Edge>,
    pub branches: HashMap<String, Uuid>,
    pub active_branch: String,
    pub history: HashMap<Uuid, Vec<NodeSnapshot>>,
}

// ── Tagged union for versioned graphs ────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "version")]
pub enum VersionedGraph {
    #[serde(rename = "2")]
    V2(V2Graph),
    #[serde(rename = "3")]
    V3(V3Graph),
}

/// Detect the version of a serialized graph by parsing only the `"version"` tag.
/// V1 files have no `version` field, so absence defaults to 1.
fn detect_version(data: &str) -> u32 {
    #[derive(Deserialize)]
    struct VersionTag {
        version: String,
    }
    serde_json::from_str::<VersionTag>(data)
        .ok()
        .and_then(|t| t.version.parse().ok())
        .unwrap_or(1)
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
            stop_reason: None,
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

/// Migrate V2 → V3: add empty history. Nodes already handle `stop_reason`
/// via `#[serde(default)]` so no per-node transformation needed.
fn migrate_v2_to_v3(v2: V2Graph) -> V3Graph {
    V3Graph {
        nodes: v2.nodes,
        edges: v2.edges,
        branches: v2.branches,
        active_branch: v2.active_branch,
        history: HashMap::new(),
    }
}

// ── Public API ───────────────────────────────────────────────────────

/// Back up a graph file before migration.
fn backup_graph(graph_path: &Path, version: u32) -> anyhow::Result<()> {
    let backup_name = format!("graph.v{version}.json.bak",);
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
        return match versioned {
            VersionedGraph::V3(v3) => v3_to_conversation_graph(v3),
            VersionedGraph::V2(_) => unreachable!("version >= 3 but deserialized as V2"),
        };
    }

    // Migration needed
    backup_graph(graph_path, version)?;

    let v3 = match version {
        1 => {
            let v1: V1Graph = serde_json::from_str(&data)?;
            migrate_v2_to_v3(migrate_v1_to_v2(v1))
        }
        2 => {
            let versioned: VersionedGraph = serde_json::from_str(&data)?;
            let VersionedGraph::V2(v2) = versioned else {
                anyhow::bail!("Expected V2 graph for version 2")
            };
            migrate_v2_to_v3(v2)
        }
        other => anyhow::bail!("Unknown graph version: {other}"),
    };

    // Write the migrated graph wrapped in the versioned envelope
    let migrated_json = serde_json::to_string_pretty(&VersionedGraph::V3(v3.clone()))?;
    let tmp_path = graph_path.with_extension("json.tmp");
    std::fs::write(&tmp_path, &migrated_json)?;
    std::fs::rename(&tmp_path, graph_path)?;

    v3_to_conversation_graph(v3)
}

/// Convert a `V3Graph` to the live `ConversationGraph`.
fn v3_to_conversation_graph(v3: V3Graph) -> anyhow::Result<ConversationGraph> {
    let raw = GraphRaw {
        nodes: v3.nodes,
        edges: v3.edges,
        branches: v3.branches,
        active_branch: v3.active_branch,
        history: v3.history,
    };

    let json = serde_json::to_string(&raw)?;
    let graph: ConversationGraph = serde_json::from_str(&json)?;
    Ok(graph)
}

/// Wrap a `ConversationGraph` in the V3 envelope for saving.
pub fn to_versioned_json(graph: &ConversationGraph) -> anyhow::Result<String> {
    let raw_json = serde_json::to_string(graph)?;
    let raw: GraphRaw = serde_json::from_str(&raw_json)?;
    let v3 = V3Graph {
        nodes: raw.nodes,
        edges: raw.edges,
        branches: raw.branches,
        active_branch: raw.active_branch,
        history: raw.history,
    };
    let versioned = VersionedGraph::V3(v3);

    Ok(serde_json::to_string_pretty(&versioned)?)
}

#[cfg(test)]
#[path = "migration_tests.rs"]
mod tests;
