use super::{sanitize_message_boundaries, truncate_messages};
use crate::graph::Role;
use crate::llm::{ChatContent, ChatMessage, ContentBlock};

/// Helper: plain text message.
fn text_msg(role: Role, text: &str) -> ChatMessage {
    ChatMessage::text(role, text)
}

/// Helper: user message containing only a `ToolResult` block.
fn tool_result_msg(tool_use_id: &str) -> ChatMessage {
    ChatMessage {
        role: Role::User,
        content: ChatContent::Blocks(vec![ContentBlock::ToolResult {
            tool_use_id: tool_use_id.to_string(),
            content: crate::graph::ToolResultContent::text("ok"),
            is_error: false,
        }]),
    }
}

/// Helper: assistant message containing a `ToolUse` block.
fn tool_use_msg(id: &str) -> ChatMessage {
    ChatMessage {
        role: Role::Assistant,
        content: ChatContent::Blocks(vec![ContentBlock::ToolUse {
            id: id.to_string(),
            name: "read_file".to_string(),
            input: crate::llm::RawJson("{}".to_string()),
        }]),
    }
}

/// Bug: `truncation` with ratio near 1.0 removes messages when it shouldn't.
/// When `token_count` barely exceeds max, the `target_chars` should still
/// preserve most messages.
#[test]
fn test_truncate_ratio_near_one_preserves_messages() {
    let mut msgs = vec![
        text_msg(Role::User, "hello"),
        text_msg(Role::Assistant, "world"),
    ];
    // max_tokens == token_count means ratio = 1.0 → target_chars == total_chars.
    truncate_messages(&mut msgs, 100, 100);
    assert_eq!(msgs.len(), 2, "ratio=1.0 should remove nothing");
}

/// Bug: orphaned `tool_result` at the front is not dropped — API rejects
/// a conversation starting with an unmatched `tool_result`.
#[test]
fn test_sanitize_drops_orphaned_tool_result_at_front() {
    let mut msgs = vec![
        tool_result_msg("toolu_123"),
        text_msg(Role::User, "hello"),
        text_msg(Role::Assistant, "hi"),
    ];
    sanitize_message_boundaries(&mut msgs);
    assert_eq!(msgs.len(), 2);
    assert_eq!(msgs[0].role, Role::User);
    assert!(
        matches!(&msgs[0].content, ChatContent::Text(t) if t == "hello"),
        "first message should be the text user message"
    );
}

/// Bug: leading assistant message not dropped — API requires user first.
#[test]
fn test_sanitize_drops_leading_assistant() {
    let mut msgs = vec![
        text_msg(Role::Assistant, "I am an assistant"),
        text_msg(Role::User, "hello"),
    ];
    sanitize_message_boundaries(&mut msgs);
    assert_eq!(msgs.len(), 1);
    assert_eq!(msgs[0].role, Role::User);
}

/// Bug: trailing assistant with `ToolUse` not popped — API rejects
/// `tool_use` without a following `tool_result`.
#[test]
fn test_sanitize_drops_trailing_tool_use_assistant() {
    let mut msgs = vec![
        text_msg(Role::User, "do something"),
        tool_use_msg("toolu_456"),
    ];
    sanitize_message_boundaries(&mut msgs);
    assert_eq!(msgs.len(), 1);
    assert_eq!(msgs[0].role, Role::User);
}

/// Bug: all three passes fail to compose — mixed violation input
/// leaves structural issues after partial cleanup.
#[test]
fn test_sanitize_composes_all_passes() {
    let mut msgs = vec![
        tool_result_msg("toolu_orphan"),      // orphaned tool result
        text_msg(Role::Assistant, "stale"),   // leading assistant
        text_msg(Role::User, "actual input"), // the real start
        text_msg(Role::Assistant, "response"),
        tool_use_msg("toolu_trailing"), // trailing tool use
    ];
    sanitize_message_boundaries(&mut msgs);
    assert_eq!(msgs.len(), 2);
    assert_eq!(msgs[0].role, Role::User);
    assert!(matches!(&msgs[0].content, ChatContent::Text(t) if t == "actual input"));
    assert_eq!(msgs[1].role, Role::Assistant);
    assert!(matches!(&msgs[1].content, ChatContent::Text(t) if t == "response"));
}

/// Bug: truncation underflows when all messages are tiny and ratio
/// is extremely small (e.g., 1 token max with 10000 tokens used).
#[test]
fn test_truncate_extreme_ratio_leaves_at_least_one() {
    let mut msgs = vec![
        text_msg(Role::User, "a"),
        text_msg(Role::Assistant, "b"),
        text_msg(Role::User, "c"),
    ];
    truncate_messages(&mut msgs, 1, 10000);
    assert!(
        !msgs.is_empty(),
        "truncation should always leave at least one message"
    );
}

/// Bug: trailing plain-text assistant (no tool use) incorrectly popped.
/// Only tool-use assistants should be dropped.
#[test]
fn test_sanitize_keeps_trailing_text_assistant() {
    let mut msgs = vec![
        text_msg(Role::User, "question"),
        text_msg(Role::Assistant, "answer"),
    ];
    sanitize_message_boundaries(&mut msgs);
    assert_eq!(msgs.len(), 2, "plain-text trailing assistant should remain");
}

// ── finalize_context integration tests ──────────────────────────────

/// Bug: `finalize_context` does not truncate when token count exceeds budget —
/// API rejects oversized requests.
#[tokio::test]
async fn test_finalize_truncates_when_over_budget() {
    use crate::llm::mock::MockLlmProvider;

    let provider = MockLlmProvider::with_token_count(200);
    let messages = vec![
        text_msg(Role::User, &"a".repeat(100)),
        text_msg(Role::Assistant, &"b".repeat(100)),
        text_msg(Role::User, &"c".repeat(100)),
    ];

    let (_, result) = super::finalize_context(None, messages, &provider, "m", 100, &[])
        .await
        .unwrap();

    assert!(
        result.len() < 3,
        "should have truncated some messages, got {}",
        result.len()
    );
}

/// Bug: `finalize_context` truncates even when under budget.
#[tokio::test]
async fn test_finalize_passthrough_when_under_budget() {
    use crate::llm::mock::MockLlmProvider;

    let provider = MockLlmProvider::with_token_count(50);
    let messages = vec![
        text_msg(Role::User, "hello"),
        text_msg(Role::Assistant, "world"),
    ];

    let (_, result) = super::finalize_context(None, messages, &provider, "m", 100, &[])
        .await
        .unwrap();

    assert_eq!(result.len(), 2, "should pass through all messages");
}

/// Bug: `finalize_context` does not sanitize after truncation — structural
/// violations left by truncation break the API request.
#[tokio::test]
async fn test_finalize_sanitizes_after_truncation() {
    use crate::llm::mock::MockLlmProvider;

    let provider = MockLlmProvider::with_token_count(200);
    // After truncation, the first message may be a tool_result — sanitization must fix this.
    let messages = vec![
        text_msg(Role::User, &"padding".repeat(50)),
        tool_result_msg("toolu_orphan"),
        text_msg(Role::User, "real message"),
        text_msg(Role::Assistant, "response"),
    ];

    let (_, result) = super::finalize_context(None, messages, &provider, "m", 100, &[])
        .await
        .unwrap();

    // After sanitization, no message should start with a tool_result.
    if !result.is_empty() {
        let first = &result[0];
        let is_tool_result = matches!(
            &first.content,
            crate::llm::ChatContent::Blocks(b)
                if b.iter().all(|b| matches!(b, crate::llm::ContentBlock::ToolResult { .. }))
        );
        assert!(
            !is_tool_result,
            "first message should not be an orphaned tool_result after sanitization"
        );
    }
}
