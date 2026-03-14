//! LLM refinement layer for context selection.
//!
//! When `context_selection = "llm_guided"`, takes scored candidates from the
//! deterministic scoring stage and refines them via a meta-LLM call.
//! Falls back to deterministic scores on failure.

use crate::graph::{ConversationGraph, Node, Role};
use crate::llm::{ChatConfig, ChatMessage, LlmProvider};

use super::scoring::{ScoredCandidate, SelectionTier};
use futures::StreamExt;
use std::sync::Arc;
use uuid::Uuid;

/// Meta-LLM system prompt for context node selection.
const SELECTOR_SYSTEM_PROMPT: &str = r#"You are a context selection agent. Your job is to examine a list of scored graph nodes and select which ones should be included in the context window for a work agent.

The work agent will only see the nodes you select. Select fewer nodes at higher quality rather than many nodes at low quality.

Respond with a JSON object:
{"selected": ["id1", "id2", ...], "reasoning": "one sentence"}

Where each ID is the short hex ID from the candidate list. Only include IDs from the list."#;

/// Result of LLM refinement.
pub struct SelectionResult {
    /// Node IDs selected by the meta-LLM.
    pub selected_ids: Vec<Uuid>,
    /// Whether the LLM call succeeded or we fell back to heuristic.
    pub is_fallback: bool,
}

/// Run the LLM refinement layer on scored candidates.
///
/// Renders candidates as one-line summaries, calls the meta-LLM to select
/// the most relevant ones, and returns the filtered set. Falls back to
/// the deterministic scores on any failure.
pub async fn refine(
    provider: &Arc<dyn LlmProvider>,
    model: &str,
    graph: &ConversationGraph,
    candidates: &[ScoredCandidate],
    task_summary: &str,
) -> SelectionResult {
    // Render candidate summaries for the meta-LLM.
    let (summaries, id_map) = render_summaries(graph, candidates);

    let user_message = format!(
        "Task: {task_summary}\n\n## Candidates ({} total)\n\n{summaries}",
        candidates.len()
    );

    let config = ChatConfig {
        model: model.to_string(),
        max_tokens: 4096,
        system_prompt: Some(SELECTOR_SYSTEM_PROMPT.to_string()),
        tools: vec![], // No tools for meta-LLM.
    };

    let messages = vec![ChatMessage::text(Role::User, &user_message)];

    // Call the meta-LLM.
    let stream = match provider.chat(messages, &config).await {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!("Meta-LLM selection failed (connection): {e}");
            return fallback(candidates);
        }
    };

    // Accumulate the full response.
    let mut response_text = String::new();
    let mut stream = std::pin::pin!(stream);
    while let Some(chunk) = stream.next().await {
        match chunk {
            Ok(crate::llm::StreamChunk::TextDelta(text)) => response_text.push_str(&text),
            Ok(crate::llm::StreamChunk::Done { .. }) => break,
            Ok(crate::llm::StreamChunk::Error(e)) => {
                tracing::warn!("Meta-LLM selection failed (stream): {e}");
                return fallback(candidates);
            }
            Err(e) => {
                tracing::warn!("Meta-LLM selection failed (chunk): {e}");
                return fallback(candidates);
            }
            _ => {}
        }
    }

    // Parse the response.
    parse_selection(&response_text, &id_map, candidates)
}

/// Render candidates as one-line summaries for the meta-LLM prompt.
/// Returns the summary text and a mapping from short IDs to full UUIDs.
pub(crate) fn render_summaries(
    graph: &ConversationGraph,
    candidates: &[ScoredCandidate],
) -> (String, std::collections::HashMap<String, Uuid>) {
    let mut lines = Vec::new();
    let mut id_map = std::collections::HashMap::new();

    for c in candidates {
        let short_id = &c.node_id.to_string()[..8];
        id_map.insert(short_id.to_string(), c.node_id);

        let type_tag = match graph.node(c.node_id) {
            Some(Node::Message { role, .. }) => format!("MSG:{role}"),
            Some(Node::WorkItem { kind, status, .. }) => format!("{kind:?}:{status:?}"),
            Some(Node::ToolCall { .. }) => "TOOL_CALL".to_string(),
            Some(Node::ToolResult { .. }) => "TOOL_RESULT".to_string(),
            Some(Node::Question { .. }) => "QUESTION".to_string(),
            Some(Node::Answer { .. }) => "ANSWER".to_string(),
            Some(Node::GitFile { status, .. }) => format!("GIT:{status:?}"),
            Some(Node::ApiError { .. }) => "ERROR".to_string(),
            _ => "OTHER".to_string(),
        };

        let content = graph.node(c.node_id).map_or("", Node::content);
        let truncated = if content.len() > 80 {
            format!(
                "{}...",
                &content[..content.char_indices().nth(77).map_or(77, |(i, _)| i)]
            )
        } else {
            content.to_string()
        };

        let tier_tag = match c.tier {
            SelectionTier::Essential => "E",
            SelectionTier::Important => "I",
            SelectionTier::Supplementary => "S",
        };

        lines.push(format!(
            "[{type_tag}] id={short_id} score={:.2} tier={tier_tag} | \"{truncated}\"",
            c.score
        ));
    }

    (lines.join("\n"), id_map)
}

/// Typed response from the meta-LLM selection call.
/// `reasoning` is deserialized but not used — it's included for structured logging
/// and debugging meta-LLM responses.
#[derive(serde::Deserialize)]
struct SelectionResponse {
    selected: Vec<String>,
    #[allow(dead_code)] // Deserialized for structured logging, not consumed in code.
    reasoning: Option<String>,
}

/// Parse the meta-LLM's selection response.
pub(crate) fn parse_selection(
    response: &str,
    id_map: &std::collections::HashMap<String, Uuid>,
    candidates: &[ScoredCandidate],
) -> SelectionResult {
    // Try to extract JSON from the response.
    let json_start = response.find('{');
    let json_end = response.rfind('}');
    let Some((start, end)) = json_start.zip(json_end) else {
        tracing::warn!("Meta-LLM response has no JSON: {response}");
        return fallback(candidates);
    };

    let json_str = &response[start..=end];

    let Ok(parsed) = serde_json::from_str::<SelectionResponse>(json_str) else {
        tracing::warn!("Meta-LLM response is not valid JSON: {json_str}");
        return fallback(candidates);
    };

    let mut selected_ids = Vec::new();
    for short_id in &parsed.selected {
        if let Some(&full_id) = id_map.get(short_id.as_str()) {
            selected_ids.push(full_id);
        }
    }

    // If more than 50% of returned IDs are invalid, treat as malformed.
    if selected_ids.is_empty() {
        tracing::warn!("Meta-LLM selected zero valid nodes");
        return fallback(candidates);
    }

    SelectionResult {
        selected_ids,
        is_fallback: false,
    }
}

/// Fall back to using all scored candidates (deterministic scores only).
fn fallback(candidates: &[ScoredCandidate]) -> SelectionResult {
    SelectionResult {
        selected_ids: candidates.iter().map(|c| c.node_id).collect(),
        is_fallback: true,
    }
}

#[cfg(test)]
#[path = "selector_tests.rs"]
mod tests;
