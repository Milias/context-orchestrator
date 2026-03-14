//! Conversational context policy — interactive chat with the user.
//!
//! Anchors on the active branch leaf, walks `RespondsTo` ancestors, includes
//! all messages verbatim. Produces identical output to the original
//! `extract_messages()` function (behavioral equivalence).

use crate::graph::tool_types::ToolCallStatus;
use crate::graph::{ConversationGraph, EdgeKind, Node, Role};
use crate::llm::{ChatContent, ChatMessage, ContentBlock, RawJson};
use uuid::Uuid;

/// Build the full message list from the graph, matching the original
/// `extract_messages()` output exactly. Includes plan section injection.
pub fn build_messages(graph: &ConversationGraph) -> (Option<String>, Vec<ChatMessage>) {
    let history = graph
        .get_branch_history(graph.active_branch())
        .unwrap_or_default();

    let mut system_prompt = None;
    let mut messages = Vec::new();

    for node in history {
        match node {
            Node::SystemDirective { content, .. } => {
                system_prompt = Some(content.clone());
            }
            Node::Message {
                id, role, content, ..
            } => match role {
                Role::System => {}
                Role::User => {
                    messages.push(ChatMessage::text(Role::User, content));
                }
                Role::Assistant => {
                    let (asst_msg, result_msgs) =
                        build_assistant_message_with_tools(graph, *id, content);
                    messages.push(asst_msg);
                    messages.extend(result_msgs);
                }
            },
            Node::WorkItem { .. }
            | Node::GitFile { .. }
            | Node::Tool { .. }
            | Node::BackgroundTask { .. }
            | Node::ThinkBlock { .. }
            | Node::ToolCall { .. }
            | Node::ToolResult { .. }
            | Node::Question { .. }
            | Node::Answer { .. } => {}
        }
    }

    // Inject active plan context into the system prompt.
    if let Some(plan_section) = crate::app::plan::context::build_plan_section(graph) {
        let prompt = system_prompt.get_or_insert_with(String::new);
        prompt.push_str("\n\n");
        prompt.push_str(&plan_section);
    }

    (system_prompt, messages)
}

/// Build assistant `ChatMessage` with `ToolUse` blocks and any following
/// user `ToolResult` messages. Ensures Anthropic API tool call/result pairing.
fn build_assistant_message_with_tools(
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
