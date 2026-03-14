use super::{sanitize_message_boundaries, sanitize_tool_pairing, truncate_messages};
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

/// Bug: `sanitize_tool_pairing` removes all blocks from a user message, dropping
/// it. This exposes a leading assistant message that `sanitize_message_boundaries`
/// already passed over. Without re-running boundaries after tool pairing, the
/// API receives an assistant-first message array.
#[test]
fn test_tool_pairing_creates_boundary_violation_caught_by_rerun() {
    // Simulate a post-truncation state where tool_pairing will drop a user message:
    // [Assistant(text), User(orphaned_tool_result), User("hello"), Assistant("response")]
    let mut msgs = vec![
        ChatMessage {
            role: Role::Assistant,
            content: ChatContent::Blocks(vec![ContentBlock::Text {
                text: "I will use a tool".to_string(),
            }]),
        },
        tool_result_msg("toolu_orphan"), // no matching tool_use
        text_msg(Role::User, "hello"),
        text_msg(Role::Assistant, "response"),
    ];
    // Run both sanitizers in the correct order.
    super::sanitize_message_boundaries(&mut msgs);
    super::sanitize_tool_pairing(&mut msgs);
    super::sanitize_message_boundaries(&mut msgs);
    // First message must be User (API requirement).
    assert_eq!(msgs[0].role, Role::User);
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

/// Bug: Pass 2 drops a leading assistant message, exposing an orphaned
/// `tool_result` user message at position 0 that Pass 1 already checked.
/// Without the outer loop, the orphan survives and causes a 400 error.
#[test]
fn test_sanitize_pass2_exposes_orphaned_tool_result() {
    let mut msgs = vec![
        tool_use_msg("toolu_a"),
        tool_result_msg("toolu_a"),
        text_msg(Role::User, "hello"),
        text_msg(Role::Assistant, "response"),
    ];
    sanitize_message_boundaries(&mut msgs);
    assert!(
        !matches!(
            &msgs[0].content,
            ChatContent::Blocks(b) if b.iter().all(|b| matches!(b, ContentBlock::ToolResult { .. }))
        ),
        "orphaned tool_result must not be at position 0"
    );
    assert_eq!(msgs[0].role, Role::User);
    assert!(matches!(&msgs[0].content, ChatContent::Text(t) if t == "hello"));
}

/// Bug: mid-conversation `tool_result` referencing a truncated `tool_use` is
/// not removed. The API rejects `tool_result` blocks whose `tool_use_id` has
/// no matching `tool_use` block anywhere in the conversation.
#[test]
fn test_sanitize_tool_pairing_orphaned_result() {
    let mut msgs = vec![
        text_msg(Role::User, "hello"),
        // tool_use for toolu_a was truncated — only result remains
        tool_result_msg("toolu_a"),
        text_msg(Role::User, "next"),
        text_msg(Role::Assistant, "response"),
    ];
    sanitize_tool_pairing(&mut msgs);
    for msg in &msgs {
        if let ChatContent::Blocks(blocks) = &msg.content {
            for block in blocks {
                assert!(
                    !matches!(block, ContentBlock::ToolResult { tool_use_id, .. } if tool_use_id == "toolu_a"),
                    "orphaned tool_result(toolu_a) should have been removed"
                );
            }
        }
    }
}

/// Bug: mid-conversation `tool_use` block whose `tool_result` was truncated
/// is not removed. The API rejects unpaired `tool_use` blocks.
#[test]
fn test_sanitize_tool_pairing_orphaned_use() {
    let mut msgs = vec![
        text_msg(Role::User, "hello"),
        tool_use_msg("toolu_b"),
        // tool_result for toolu_b was truncated
        text_msg(Role::User, "next"),
        text_msg(Role::Assistant, "response"),
    ];
    sanitize_tool_pairing(&mut msgs);
    for msg in &msgs {
        if let ChatContent::Blocks(blocks) = &msg.content {
            for block in blocks {
                assert!(
                    !matches!(block, ContentBlock::ToolUse { id, .. } if id == "toolu_b"),
                    "orphaned tool_use(toolu_b) should have been removed"
                );
            }
        }
    }
}

/// Bug: `sanitize_tool_pairing` removes valid pairs because it confuses
/// paired and orphaned blocks. Valid pairs must survive untouched.
#[test]
fn test_sanitize_tool_pairing_keeps_valid_pairs() {
    let mut msgs = vec![
        text_msg(Role::User, "hello"),
        tool_use_msg("toolu_ok"),
        tool_result_msg("toolu_ok"),
        text_msg(Role::User, "next"),
    ];
    let len_before = msgs.len();
    sanitize_tool_pairing(&mut msgs);
    assert_eq!(msgs.len(), len_before, "valid pairs should not be removed");
}

/// Bug: message containing a mix of valid and orphaned tool blocks has
/// the orphan removed but the valid block preserved.
#[test]
fn test_sanitize_tool_pairing_mixed_blocks() {
    let mut msgs = vec![
        text_msg(Role::User, "hello"),
        // Assistant with two tool_use: one paired, one orphaned
        ChatMessage {
            role: Role::Assistant,
            content: ChatContent::Blocks(vec![
                ContentBlock::ToolUse {
                    id: "toolu_paired".to_string(),
                    name: "read_file".to_string(),
                    input: crate::llm::RawJson("{}".to_string()),
                },
                ContentBlock::ToolUse {
                    id: "toolu_orphan".to_string(),
                    name: "read_file".to_string(),
                    input: crate::llm::RawJson("{}".to_string()),
                },
            ]),
        },
        tool_result_msg("toolu_paired"),
    ];
    sanitize_tool_pairing(&mut msgs);
    // The paired tool_use should remain, the orphan should be gone
    if let ChatContent::Blocks(blocks) = &msgs[1].content {
        assert_eq!(blocks.len(), 1, "only paired tool_use should remain");
        assert!(matches!(&blocks[0], ContentBlock::ToolUse { id, .. } if id == "toolu_paired"));
    } else {
        panic!("assistant message should still have blocks");
    }
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
