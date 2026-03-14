//! Context pipeline: graph → purpose-specific LLM context windows.
//!
//! The pipeline has 6 stages configured by a [`ContextPolicy`]:
//! 1. **Anchor** — where traversal starts
//! 2. **Expand** — which nodes to gather
//! 3. **Score** — relevance ranking
//! 4. **Budget** — token allocation per section
//! 5. **Render** — serialization to chat messages
//! 6. **Sanitize** — API constraint enforcement

pub mod budget;
pub mod candidates;
pub(crate) mod error_context;
pub mod policies;
pub mod sanitize;
pub mod scoring;
pub mod selector;

use crate::graph::ConversationGraph;
use crate::llm::{ChatMessage, LlmProvider, ToolDefinition};
use uuid::Uuid;

/// Extract messages from the conversation graph using a context policy.
/// Synchronous — no API calls. Caller should hold a read lock on the shared graph.
pub fn extract_messages(
    graph: &ConversationGraph,
    policy: &policies::ContextPolicy,
    agent_id: Uuid,
) -> policies::ContextBuildResult {
    policy.build_context(graph, agent_id)
}

/// Legacy extraction for backward compatibility (uses conversational policy).
/// Synchronous — no API calls. Caller should hold a read lock on the shared graph.
pub fn extract_messages_conversational(
    graph: &ConversationGraph,
    agent_id: Option<Uuid>,
) -> (Option<String>, Vec<ChatMessage>) {
    policies::conversational::build_messages(graph, agent_id)
}

/// Count tokens and truncate messages if needed. Async — calls the LLM provider API.
/// Must NOT hold any graph lock while calling this.
pub async fn finalize_context(
    system_prompt: Option<String>,
    messages: Vec<ChatMessage>,
    provider: &dyn LlmProvider,
    model: &str,
    max_context_tokens: u32,
    tools: &[ToolDefinition],
) -> anyhow::Result<(Option<String>, Vec<ChatMessage>)> {
    sanitize::finalize_context(
        system_prompt,
        messages,
        provider,
        model,
        max_context_tokens,
        tools,
    )
    .await
}
