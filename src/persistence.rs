use crate::graph::ConversationGraph;
use crate::migration;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversationMetadata {
    pub id: String,
    pub name: String,
    pub created_at: DateTime<Utc>,
    pub last_modified: DateTime<Utc>,
}

fn default_conversations_dir() -> anyhow::Result<PathBuf> {
    let home =
        std::env::var("HOME").map_err(|_| anyhow::anyhow!("HOME environment variable not set"))?;
    Ok(PathBuf::from(home)
        .join(".context-manager")
        .join("conversations"))
}

fn save_conversation_to(
    base: &Path,
    conversation_id: &str,
    metadata: &ConversationMetadata,
    graph: &ConversationGraph,
) -> anyhow::Result<()> {
    let dir = base.join(conversation_id);
    std::fs::create_dir_all(&dir)?;

    let graph_path = dir.join("graph.json");
    let graph_tmp = dir.join("graph.json.tmp");
    std::fs::write(&graph_tmp, migration::to_versioned_json(graph)?)?;
    std::fs::rename(&graph_tmp, &graph_path)?;

    let meta_path = dir.join("metadata.json");
    let meta_tmp = dir.join("metadata.json.tmp");
    std::fs::write(&meta_tmp, serde_json::to_string_pretty(metadata)?)?;
    std::fs::rename(&meta_tmp, &meta_path)?;

    Ok(())
}

fn load_conversation_from(
    base: &Path,
    conversation_id: &str,
) -> anyhow::Result<(ConversationMetadata, ConversationGraph)> {
    let dir = base.join(conversation_id);

    let graph_path = dir.join("graph.json");
    let graph = migration::load_and_migrate(&graph_path)?;

    let metadata_str = std::fs::read_to_string(dir.join("metadata.json"))?;
    let metadata: ConversationMetadata = serde_json::from_str(&metadata_str)?;

    Ok((metadata, graph))
}

fn list_conversations_in(base: &Path) -> anyhow::Result<Vec<ConversationMetadata>> {
    if !base.exists() {
        return Ok(Vec::new());
    }

    let mut conversations = Vec::new();
    for entry in std::fs::read_dir(base)? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let meta_path = entry.path().join("metadata.json");
        if let Ok(data) = std::fs::read_to_string(&meta_path) {
            if let Ok(metadata) = serde_json::from_str::<ConversationMetadata>(&data) {
                conversations.push(metadata);
            }
        }
    }

    conversations.sort_by(|a, b| b.last_modified.cmp(&a.last_modified));
    Ok(conversations)
}

pub fn save_conversation(
    conversation_id: &str,
    metadata: &ConversationMetadata,
    graph: &ConversationGraph,
) -> anyhow::Result<()> {
    save_conversation_to(
        &default_conversations_dir()?,
        conversation_id,
        metadata,
        graph,
    )
}

pub fn load_conversation(
    conversation_id: &str,
) -> anyhow::Result<(ConversationMetadata, ConversationGraph)> {
    load_conversation_from(&default_conversations_dir()?, conversation_id)
}

pub fn list_conversations() -> anyhow::Result<Vec<ConversationMetadata>> {
    list_conversations_in(&default_conversations_dir()?)
}

#[cfg(test)]
#[path = "persistence_tests.rs"]
mod tests;
