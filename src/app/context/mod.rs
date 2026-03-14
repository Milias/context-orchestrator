//! Context pipeline: graph → purpose-specific LLM context windows.
//!
//! The pipeline has 6 stages configured by a [`ContextPolicy`]:
//! 1. **Anchor** — where traversal starts
//! 2. **Expand** — which nodes to gather
//! 3. **Score** — relevance ranking
//! 4. **Budget** — token allocation per section
//! 5. **Render** — serialization to chat messages
//! 6. **Sanitize** — API constraint enforcement

pub mod policies;
pub mod sanitize;

use crate::graph::ConversationGraph;
use crate::llm::{ChatMessage, LlmProvider, ToolDefinition};

/// Extract messages from the conversation graph using the `ConversationalPolicy`.
/// Synchronous — no API calls. Caller should hold a read lock on the shared graph.
///
/// This is the compatibility entry point that produces identical output to the
/// original monolithic `extract_messages()` function.
pub fn extract_messages(
    graph: &ConversationGraph,
    _tools: &[ToolDefinition],
) -> (Option<String>, Vec<ChatMessage>) {
    policies::conversational::build_messages(graph)
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
