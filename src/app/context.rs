use crate::graph::tool_types::ToolCallStatus;
use crate::graph::{ConversationGraph, EdgeKind, Node, Role};
use crate::llm::{ChatContent, ChatMessage, ContentBlock, LlmProvider, RawJson, ToolDefinition};

/// Extract messages from the conversation graph. Synchronous — no API calls.
/// Caller should hold a read lock on the shared graph while calling this.
pub(super) fn extract_messages(
    graph: &ConversationGraph,
    tools: &[ToolDefinition],
) -> (Option<String>, Vec<ChatMessage>) {
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
                    messages.push(ChatMessage::text("user", content));
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
            | Node::ToolResult { .. } => {}
        }
    }

    // Append tool names to system prompt so the LLM knows what's available.
    if !tools.is_empty() {
        // No mutation needed — tool definitions are passed separately to the API.
    }

    (system_prompt, messages)
}

/// Count tokens and truncate messages if needed. Async — calls the LLM provider API.
/// Must NOT hold any graph lock while calling this.
pub(super) async fn finalize_context(
    system_prompt: Option<String>,
    mut messages: Vec<ChatMessage>,
    provider: &dyn LlmProvider,
    model: &str,
    max_context_tokens: u32,
    tools: &[ToolDefinition],
) -> anyhow::Result<(Option<String>, Vec<ChatMessage>)> {
    let token_count = provider
        .count_tokens(&messages, model, system_prompt.as_deref(), tools)
        .await
        .unwrap_or_default();

    if token_count > max_context_tokens {
        truncate_messages(&mut messages, max_context_tokens, token_count);
    }

    sanitize_message_boundaries(&mut messages);

    Ok((system_prompt, messages))
}

/// Build assistant `ChatMessage` with `ToolUse` blocks and any following
/// user `ToolResult` messages. Ensures Anthropic API tool call/result pairing.
pub(super) fn build_assistant_message_with_tools(
    graph: &ConversationGraph,
    message_id: uuid::Uuid,
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
        return (ChatMessage::text("assistant", text_content), vec![]);
    }
    let mut blocks = Vec::new();
    if !text_content.is_empty() {
        blocks.push(ContentBlock::Text {
            text: text_content.to_string(),
        });
    }
    blocks.extend(tool_use_blocks);
    let asst = ChatMessage {
        role: "assistant".to_string(),
        content: ChatContent::Blocks(blocks),
    };
    let results = ChatMessage {
        role: "user".to_string(),
        content: ChatContent::Blocks(result_blocks),
    };
    (asst, vec![results])
}

fn truncate_messages(messages: &mut Vec<ChatMessage>, max_tokens: u32, token_count: u32) {
    let total_chars: usize = messages.iter().map(|m| m.content.char_len()).sum();
    let ratio = f64::from(max_tokens) / f64::from(token_count);
    // Truncation/sign-loss/precision-loss are acceptable here: total_chars and ratio
    // are both non-negative and the result fits comfortably in usize for any realistic
    // conversation size.
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss
    )]
    let target_chars = (total_chars as f64 * ratio) as usize;

    let mut current_chars = total_chars;
    let mut remove_count = 0;
    while current_chars > target_chars && remove_count < messages.len() - 1 {
        current_chars -= messages[remove_count].content.char_len();
        remove_count += 1;
    }
    if remove_count > 0 {
        messages.drain(0..remove_count);
    }
}

fn sanitize_message_boundaries(messages: &mut Vec<ChatMessage>) {
    // Drop orphaned tool_result user messages at the front after truncation.
    while messages.len() > 1 && messages[0].role == "user" {
        let all_results = matches!(&messages[0].content,
            ChatContent::Blocks(b) if b.iter().all(|b| matches!(b, ContentBlock::ToolResult { .. })));
        if all_results {
            messages.remove(0);
        } else {
            break;
        }
    }

    // Drop leading assistant messages — API requires conversation start with user.
    while messages.len() > 1 && messages[0].role == "assistant" {
        messages.remove(0);
    }

    // Drop trailing assistant messages with tool_use blocks that lack a following tool_result.
    while messages.len() > 1 {
        let dominated = messages.last().is_some_and(|last| {
            last.role == "assistant"
                && matches!(&last.content,
                    ChatContent::Blocks(b) if b.iter().any(|b| matches!(b, ContentBlock::ToolUse { .. })))
        });
        if dominated {
            messages.pop();
        } else {
            break;
        }
    }
}
