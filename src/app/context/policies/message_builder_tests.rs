use super::*;

use crate::graph::tool_types::{ToolCallArguments, ToolCallStatus};
use crate::graph::{ConversationGraph, Node, Role, ToolResultContent};
use crate::llm::{ChatContent, ContentBlock};
use chrono::Utc;
use uuid::Uuid;

/// Helper: create a graph with an assistant message on the main branch.
fn graph_with_assistant_msg() -> (ConversationGraph, Uuid) {
    let mut graph = ConversationGraph::new("sys");
    let root = graph.branch_leaf("main").unwrap();
    let asst = Node::Message {
        id: Uuid::new_v4(),
        role: Role::Assistant,
        content: "I will help.".to_string(),
        created_at: Utc::now(),
        model: Some("claude".to_string()),
        input_tokens: None,
        output_tokens: None,
        stop_reason: None,
    };
    let asst_id = graph.add_message(root, asst).unwrap();
    (graph, asst_id)
}

/// Bug: `build_assistant_message_with_tools` includes Pending or Running tool
/// calls in the output. The Anthropic API requires every `tool_use` block to
/// have a matching `tool_result` in a subsequent user message. If Pending/Running
/// tool calls (which have no result yet) are included, the API rejects the
/// request with a pairing error. Only Completed/Failed calls with results should
/// appear.
#[test]
fn only_completed_and_failed_tool_calls_appear_in_output() {
    let (mut graph, asst_id) = graph_with_assistant_msg();

    // Add a Completed tool call with a result.
    let completed_tc = graph.add_tool_call(
        Uuid::new_v4(),
        asst_id,
        ToolCallArguments::ReadFile {
            path: "/foo.rs".to_string(),
        },
        Some("toolu_completed".to_string()),
    );
    graph
        .update_tool_call_status(completed_tc, ToolCallStatus::Completed, Some(Utc::now()))
        .unwrap();
    graph.add_tool_result(completed_tc, ToolResultContent::text("file content"), false);

    // Add a Failed tool call with a result.
    let failed_tc = graph.add_tool_call(
        Uuid::new_v4(),
        asst_id,
        ToolCallArguments::ReadFile {
            path: "/missing.rs".to_string(),
        },
        Some("toolu_failed".to_string()),
    );
    graph
        .update_tool_call_status(failed_tc, ToolCallStatus::Failed, Some(Utc::now()))
        .unwrap();
    graph.add_tool_result(failed_tc, ToolResultContent::text("not found"), true);

    // Add a Pending tool call (still running, no result).
    let pending_tc_id = Uuid::new_v4();
    let pending_tc_node = Node::ToolCall {
        id: pending_tc_id,
        api_tool_use_id: Some("toolu_pending".to_string()),
        arguments: ToolCallArguments::ReadFile {
            path: "/pending.rs".to_string(),
        },
        status: ToolCallStatus::Pending,
        parent_message_id: asst_id,
        created_at: Utc::now(),
        completed_at: None,
    };
    graph.add_node(pending_tc_node);
    let _ = graph.add_edge(pending_tc_id, asst_id, crate::graph::EdgeKind::Invoked);

    let (asst_msg, result_msgs) =
        build_assistant_message_with_tools(&graph, asst_id, "I will help.");

    // The assistant message should have tool_use blocks for completed + failed only.
    let tool_use_ids: Vec<&str> = match &asst_msg.content {
        ChatContent::Blocks(blocks) => blocks
            .iter()
            .filter_map(|b| match b {
                ContentBlock::ToolUse { id, .. } => Some(id.as_str()),
                _ => None,
            })
            .collect(),
        ChatContent::Text(_) => panic!("expected Blocks content with tool uses"),
    };

    assert!(
        tool_use_ids.contains(&"toolu_completed"),
        "completed tool call should appear in output"
    );
    assert!(
        tool_use_ids.contains(&"toolu_failed"),
        "failed tool call should appear in output"
    );
    assert!(
        !tool_use_ids.contains(&"toolu_pending"),
        "pending tool call must NOT appear in output — API requires tool_result pairing"
    );

    // Result messages should exist for the completed/failed calls.
    assert_eq!(
        result_msgs.len(),
        1,
        "should have exactly one user message with tool_result blocks"
    );
}
