//! Message boundary sanitization and truncation.
//!
//! These functions enforce Anthropic API structural constraints after
//! context truncation: no orphaned tool results, no leading assistant
//! messages, no trailing tool-use without results.

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
/// - Drop orphaned tool result user messages at the front
/// - Drop leading assistant messages (API requires user first)
/// - Drop trailing assistant messages with uncompleted tool uses
pub(crate) fn sanitize_message_boundaries(messages: &mut Vec<ChatMessage>) {
    // Drop orphaned tool_result user messages at the front after truncation.
    while messages.len() > 1 && messages[0].role == Role::User {
        let all_results = matches!(&messages[0].content,
            ChatContent::Blocks(b) if b.iter().all(|b| matches!(b, ContentBlock::ToolResult { .. })));
        if all_results {
            messages.remove(0);
        } else {
            break;
        }
    }

    // Drop leading assistant messages — API requires conversation start with user.
    while messages.len() > 1 && messages[0].role == Role::Assistant {
        messages.remove(0);
    }

    // Drop trailing assistant messages with tool_use blocks that lack a following tool_result.
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
}

#[cfg(test)]
#[path = "sanitize_tests.rs"]
mod tests;
