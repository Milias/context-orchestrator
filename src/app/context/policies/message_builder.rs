//! Shared assistant message builder for context policies.
//!
//! Constructs assistant `ChatMessage`s with paired `ToolUse`/`ToolResult` blocks
//! from graph nodes. Used by both conversational and task execution policies.

use crate::graph::tool_types::ToolCallStatus;
use crate::graph::{ConversationGraph, EdgeKind, Node, Role};
use crate::llm::{ChatContent, ChatMessage, ContentBlock, RawJson};
use uuid::Uuid;

/// Build an assistant `ChatMessage` with `ToolUse` blocks and any following
/// user `ToolResult` messages. Ensures Anthropic API tool call/result pairing.
///
/// Only includes tool calls with `Completed` or `Failed` status that have an
/// associated `ToolResult` node. Running/pending tool calls are excluded to
/// prevent API rejection (orphaned `tool_use` without matching `tool_result`).
pub(crate) fn build_assistant_message_with_tools(
    graph: &ConversationGraph,
    message_id: Uuid,
    text_content: &str,
) -> (ChatMessage, Vec<ChatMessage>) {
    let tool_call_ids = graph.sources_by_edge(message_id, EdgeKind::Invoked);
    let mut tool_use_blocks = Vec::new();
    let mut result_blocks = Vec::new();
    for tc_id in &tool_call_ids {
        let Some(Node::ToolCall {
            status,
            arguments,
            api_tool_use_id,
            ..
        }) = graph.node(*tc_id)
        else {
            continue;
        };
        if *status != ToolCallStatus::Completed && *status != ToolCallStatus::Failed {
            continue;
        }

        let result_id = graph
            .sources_by_edge(*tc_id, EdgeKind::Produced)
            .into_iter()
            .next();
        let Some(result_id) = result_id else {
            continue;
        };
        let Some(Node::ToolResult {
            content, is_error, ..
        }) = graph.node(result_id)
        else {
            continue;
        };
        let use_id = api_tool_use_id.clone().unwrap_or_else(|| tc_id.to_string());
        tool_use_blocks.push(ContentBlock::ToolUse {
            id: use_id.clone(),
            name: arguments.tool_name().to_string(),
            input: RawJson(arguments.to_input_json()),
        });
        result_blocks.push(ContentBlock::ToolResult {
            tool_use_id: use_id,
            content: content.clone(),
            is_error: *is_error,
        });
    }

    if tool_use_blocks.is_empty() {
        return (ChatMessage::text(Role::Assistant, text_content), vec![]);
    }
    let mut blocks = Vec::new();
    if !text_content.is_empty() {
        blocks.push(ContentBlock::Text {
            text: text_content.to_string(),
        });
    }
    blocks.extend(tool_use_blocks);
    let asst = ChatMessage {
        role: Role::Assistant,
        content: ChatContent::Blocks(blocks),
    };
    let results = ChatMessage {
        role: Role::User,
        content: ChatContent::Blocks(result_blocks),
    };
    (asst, vec![results])
}

#[cfg(test)]
#[path = "message_builder_tests.rs"]
mod tests;
