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
mod tests {
    use super::*;
    use crate::graph::ConversationGraph;

    #[test]
    fn test_save_and_load_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path();

        let graph = ConversationGraph::new("Test prompt");
        let metadata = ConversationMetadata {
            id: "test-conv-1".to_string(),
            name: "Test Conversation".to_string(),
            created_at: Utc::now(),
            last_modified: Utc::now(),
        };

        save_conversation_to(base, "test-conv-1", &metadata, &graph).unwrap();
        let (loaded_meta, loaded_graph) = load_conversation_from(base, "test-conv-1").unwrap();

        assert_eq!(loaded_meta.id, "test-conv-1");
        assert_eq!(loaded_meta.name, "Test Conversation");

        let orig_history = graph.get_branch_history("main").unwrap();
        let loaded_history = loaded_graph.get_branch_history("main").unwrap();
        assert_eq!(orig_history.len(), loaded_history.len());
        assert_eq!(orig_history[0].content(), loaded_history[0].content());
    }

    #[test]
    fn test_list_conversations() {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path();

        for i in 0..3 {
            let graph = ConversationGraph::new("Prompt");
            let metadata = ConversationMetadata {
                id: format!("list-test-conv-{i}"),
                name: format!("Conversation {i}"),
                created_at: Utc::now(),
                last_modified: Utc::now(),
            };
            save_conversation_to(base, &metadata.id, &metadata, &graph).unwrap();
        }

        let list = list_conversations_in(base).unwrap();
        assert_eq!(list.len(), 3);
    }

    #[test]
    fn test_load_nonexistent_errors() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(load_conversation_from(tmp.path(), "does-not-exist").is_err());
    }
}
