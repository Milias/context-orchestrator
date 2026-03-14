//! Message boundary sanitization and truncation.
//!
//! These functions enforce Anthropic API structural constraints after
//! context truncation: no orphaned tool results, no leading assistant
//! messages, no trailing tool-use without results.

use std::collections::HashSet;

use crate::graph::Role;
use crate::llm::{ChatContent, ChatMessage, ContentBlock, LlmProvider, ToolDefinition};

/// Count tokens and truncate messages if needed. Async — calls the LLM provider API.
/// Must NOT hold any graph lock while calling this.
pub async fn finalize_context(
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
    sanitize_tool_pairing(&mut messages);
    // Tool pairing can drop empty messages, creating new boundary violations.
    sanitize_message_boundaries(&mut messages);

    Ok((system_prompt, messages))
}

/// Remove oldest messages until the token budget is satisfied.
pub(crate) fn truncate_messages(
    messages: &mut Vec<ChatMessage>,
    max_tokens: u32,
    token_count: u32,
) {
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

/// Fix structural violations after truncation.
///
/// Runs three passes in a loop until stable:
/// 1. Drop orphaned `tool_result` user messages at the front
/// 2. Drop leading assistant messages (API requires user first)
/// 3. Drop trailing assistant messages with uncompleted tool uses
///
/// The loop is necessary because Pass 2 can expose new orphaned `tool_result`
/// messages at position 0 that Pass 1 already checked.
pub(crate) fn sanitize_message_boundaries(messages: &mut Vec<ChatMessage>) {
    loop {
        let len_before = messages.len();

        // Pass 1: Drop orphaned tool_result user messages at the front.
        while messages.len() > 1 && messages[0].role == Role::User {
            let all_results = matches!(&messages[0].content,
                ChatContent::Blocks(b) if b.iter().all(|b| matches!(b, ContentBlock::ToolResult { .. })));
            if all_results {
                messages.remove(0);
            } else {
                break;
            }
        }

        // Pass 2: Drop leading assistant messages — API requires user first.
        while messages.len() > 1 && messages[0].role == Role::Assistant {
            messages.remove(0);
        }

        // Pass 3: Drop trailing assistant messages with tool_use blocks lacking a result.
        while messages.len() > 1 {
            let dominated = messages.last().is_some_and(|last| {
                last.role == Role::Assistant
                    && matches!(&last.content,
                        ChatContent::Blocks(b) if b.iter().any(|b| matches!(b, ContentBlock::ToolUse { .. })))
            });
            if dominated {
                messages.pop();
            } else {
                break;
            }
        }

        if messages.len() == len_before {
            break;
        }
    }
}

/// Remove mid-conversation `tool_use` and `tool_result` blocks whose pair is missing.
///
/// After truncation, a `tool_use` block can exist without a matching `tool_result`
/// (or vice versa) anywhere in the conversation. The Anthropic API rejects these
/// orphaned blocks. This function removes them and drops any messages that become
/// empty as a result.
pub(crate) fn sanitize_tool_pairing(messages: &mut Vec<ChatMessage>) {
    let mut tool_use_ids: HashSet<String> = HashSet::new();
    let mut tool_result_ids: HashSet<String> = HashSet::new();

    for msg in messages.iter() {
        if let ChatContent::Blocks(blocks) = &msg.content {
            for block in blocks {
                match block {
                    ContentBlock::ToolUse { id, .. } => {
                        tool_use_ids.insert(id.clone());
                    }
                    ContentBlock::ToolResult { tool_use_id, .. } => {
                        tool_result_ids.insert(tool_use_id.clone());
                    }
                    ContentBlock::Text { .. } => {}
                }
            }
        }
    }

    // IDs that appear in both sets are valid pairs.
    let orphaned_results: HashSet<&String> = tool_result_ids.difference(&tool_use_ids).collect();
    let orphaned_uses: HashSet<&String> = tool_use_ids.difference(&tool_result_ids).collect();

    if orphaned_results.is_empty() && orphaned_uses.is_empty() {
        return;
    }

    // Strip orphaned blocks from messages; drop messages that become empty.
    for msg in messages.iter_mut() {
        if let ChatContent::Blocks(blocks) = &mut msg.content {
            blocks.retain(|block| match block {
                ContentBlock::ToolResult { tool_use_id, .. } => {
                    !orphaned_results.contains(tool_use_id)
                }
                ContentBlock::ToolUse { id, .. } => !orphaned_uses.contains(id),
                ContentBlock::Text { .. } => true,
            });
        }
    }

    messages.retain(|msg| match &msg.content {
        ChatContent::Text(t) => !t.is_empty(),
        ChatContent::Blocks(b) => !b.is_empty(),
    });
}

#[cfg(test)]
#[path = "sanitize_tests.rs"]
mod tests;
