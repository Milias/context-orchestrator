use crate::graph::tool_types::{ToolCallArguments, ToolCallStatus, ToolResultContent};
use crate::graph::{ConversationGraph, Node, Role};
use crate::llm::{ChatContent, ContentBlock};
use chrono::Utc;
use uuid::Uuid;

/// Helper: create a graph with a system directive and a user message.
fn graph_with_user_message(sys: &str, user_text: &str) -> ConversationGraph {
    let mut graph = ConversationGraph::new(sys);
    let root = graph.branch_leaf("main").unwrap();
    let msg = Node::Message {
        id: Uuid::new_v4(),
        role: Role::User,
        content: user_text.to_string(),
        created_at: Utc::now(),
        model: None,
        input_tokens: None,
        output_tokens: None,
        stop_reason: None,
    };
    graph.add_message(root, msg).unwrap();
    graph
}

/// Bug: `SystemDirective` content appears in `messages` instead of
/// `system_prompt`, causing the API to see it as a conversation turn.
#[test]
fn test_system_directive_goes_to_system_prompt() {
    let graph = graph_with_user_message("You are helpful", "Hello");
    let result = super::build_context(&graph, uuid::Uuid::nil());
    let (system_prompt, messages) = (result.system_prompt, result.messages);

    assert_eq!(system_prompt.as_deref(), Some("You are helpful"));
    // Messages should not contain the system directive text.
    for msg in &messages {
        if let ChatContent::Text(t) = &msg.content {
            assert_ne!(t, "You are helpful", "system text leaked into messages");
        }
    }
}

/// Bug: assistant message with no tool calls produces a `Blocks`
/// content type instead of plain `Text`, confusing downstream consumers.
#[test]
fn test_plain_assistant_produces_text_content() {
    let mut graph = graph_with_user_message("sys", "hi");
    let leaf = graph.branch_leaf("main").unwrap();
    let asst = Node::Message {
        id: Uuid::new_v4(),
        role: Role::Assistant,
        content: "Hello back!".to_string(),
        created_at: Utc::now(),
        model: None,
        input_tokens: None,
        output_tokens: None,
        stop_reason: None,
    };
    graph.add_message(leaf, asst).unwrap();

    let result = super::build_context(&graph, uuid::Uuid::nil());
    let messages = result.messages;
    let asst_msg = messages.iter().find(|m| m.role == Role::Assistant).unwrap();
    assert!(
        matches!(&asst_msg.content, ChatContent::Text(t) if t == "Hello back!"),
        "plain assistant should be ChatContent::Text, got {:?}",
        asst_msg.content
    );
}

/// Bug: tool call with `Running` status included in assistant blocks —
/// API rejects `tool_use` without a matching `tool_result`.
#[test]
fn test_running_tool_call_excluded_from_messages() {
    let mut graph = graph_with_user_message("sys", "do something");
    let leaf = graph.branch_leaf("main").unwrap();
    let asst_id = Uuid::new_v4();
    let asst = Node::Message {
        id: asst_id,
        role: Role::Assistant,
        content: "I'll read a file".to_string(),
        created_at: Utc::now(),
        model: None,
        input_tokens: None,
        output_tokens: None,
        stop_reason: None,
    };
    graph.add_message(leaf, asst).unwrap();

    // Add a Running tool call (no result yet).
    graph.add_tool_call(
        Uuid::new_v4(),
        asst_id,
        ToolCallArguments::ReadFile {
            path: "src/main.rs".to_string(),
        },
        Some("toolu_running".to_string()),
    );

    let result = super::build_context(&graph, uuid::Uuid::nil());
    let messages = result.messages;

    // The assistant message should be plain text (no ToolUse blocks).
    let asst_msg = messages.iter().find(|m| m.role == Role::Assistant).unwrap();
    match &asst_msg.content {
        ChatContent::Text(_) => {} // correct: Running tool excluded
        ChatContent::Blocks(blocks) => {
            let has_tool_use = blocks
                .iter()
                .any(|b| matches!(b, ContentBlock::ToolUse { .. }));
            assert!(
                !has_tool_use,
                "Running tool call should not appear as ToolUse block"
            );
        }
    }
}

/// Bug: completed tool call missing from assistant blocks — API gets
/// a `tool_result` without a preceding `tool_use`.
#[test]
fn test_completed_tool_call_included_in_messages() {
    let mut graph = graph_with_user_message("sys", "read main.rs");
    let leaf = graph.branch_leaf("main").unwrap();
    let asst_id = Uuid::new_v4();
    let asst = Node::Message {
        id: asst_id,
        role: Role::Assistant,
        content: String::new(),
        created_at: Utc::now(),
        model: None,
        input_tokens: None,
        output_tokens: None,
        stop_reason: None,
    };
    graph.add_message(leaf, asst).unwrap();

    let tc_id = graph.add_tool_call(
        Uuid::new_v4(),
        asst_id,
        ToolCallArguments::ReadFile {
            path: "src/main.rs".to_string(),
        },
        Some("toolu_done".to_string()),
    );
    graph
        .update_tool_call_status(tc_id, ToolCallStatus::Completed, Some(Utc::now()))
        .unwrap();
    graph.add_tool_result(tc_id, ToolResultContent::text("fn main() {}"), false);

    let result = super::build_context(&graph, uuid::Uuid::nil());
    let messages = result.messages;

    // Should have: user msg, assistant with ToolUse, user with ToolResult.
    assert!(
        messages.len() >= 3,
        "expected at least 3 messages (user, assistant+tool_use, user+tool_result), got {}",
        messages.len()
    );

    let asst_msg = &messages[1];
    assert_eq!(asst_msg.role, Role::Assistant);
    match &asst_msg.content {
        ChatContent::Blocks(blocks) => {
            let has_tool_use = blocks
                .iter()
                .any(|b| matches!(b, ContentBlock::ToolUse { .. }));
            assert!(has_tool_use, "completed tool should appear as ToolUse");
        }
        ChatContent::Text(t) => panic!("expected Blocks, got Text({t:?})"),
    }
}

/// Bug: user messages appear as assistant or are dropped entirely.
#[test]
fn test_user_message_preserved() {
    let graph = graph_with_user_message("sys", "Hello world");
    let result = super::build_context(&graph, uuid::Uuid::nil());
    let messages = result.messages;

    assert!(!messages.is_empty(), "should have at least one message");
    assert_eq!(messages[0].role, Role::User);
    assert!(
        matches!(&messages[0].content, ChatContent::Text(t) if t == "Hello world"),
        "user message content should be preserved"
    );
}
