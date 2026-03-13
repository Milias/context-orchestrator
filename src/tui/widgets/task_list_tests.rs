use super::*;
use crate::graph::tool_types::{ToolCallArguments, ToolCallStatus};
use crate::graph::{BackgroundTaskKind, ConversationGraph, Node, TaskStatus};
use chrono::{Duration as ChronoDuration, Utc};
use uuid::Uuid;

#[test]
fn active_before_completed() {
    // If partition is wrong, running tasks could sort after completed ones,
    // hiding active work from the user.
    let now = Utc::now();
    let mut graph = ConversationGraph::new("");

    let running = Node::ToolCall {
        id: Uuid::new_v4(),
        api_tool_use_id: None,
        arguments: ToolCallArguments::ReadFile {
            path: "a.rs".into(),
        },
        status: ToolCallStatus::Running,
        parent_message_id: Uuid::new_v4(),
        created_at: now - ChronoDuration::seconds(3),
        completed_at: None,
    };
    let completed = Node::ToolCall {
        id: Uuid::new_v4(),
        api_tool_use_id: None,
        arguments: ToolCallArguments::ReadFile {
            path: "b.rs".into(),
        },
        status: ToolCallStatus::Completed,
        parent_message_id: Uuid::new_v4(),
        created_at: now - ChronoDuration::seconds(5),
        completed_at: Some(now - ChronoDuration::seconds(4)),
    };
    graph.add_node(completed);
    graph.add_node(running.clone());

    let mut active = Vec::new();
    let mut history = Vec::new();
    for node in graph.nodes_by(is_task_node) {
        if let Some(entry) = TaskEntry::from_node(node, now) {
            if entry.is_active {
                active.push(entry);
            } else {
                history.push(entry);
            }
        }
    }

    assert_eq!(active.len(), 1, "should have 1 active entry");
    assert_eq!(history.len(), 1, "should have 1 history entry");
    assert!(active[0].is_active);
    assert!(!history[0].is_active);
}

#[test]
fn duration_formatting_edge_cases() {
    // Sub-second, boundary at 60s, and pending must all render correctly.
    // Wrong format strings would show "0s" instead of "5ms" or crash on large values.
    assert_eq!(format_duration(&TaskDuration::Pending), "···");
    assert_eq!(
        format_duration(&TaskDuration::Elapsed(Duration::from_millis(5))),
        "5ms"
    );
    assert_eq!(
        format_duration(&TaskDuration::Elapsed(Duration::from_millis(300))),
        "300ms"
    );
    assert_eq!(
        format_duration(&TaskDuration::Finished(Duration::from_millis(9999))),
        "10.0s"
    );
    assert_eq!(
        format_duration(&TaskDuration::Finished(Duration::from_secs(23))),
        "23s"
    );
    assert_eq!(
        format_duration(&TaskDuration::Finished(Duration::from_secs(61))),
        "1m 01s"
    );
}

#[test]
fn tool_results_excluded() {
    // If ToolResult nodes leak into the task list, every completed tool call
    // would appear twice — once as ToolCall and once as ToolResult.
    let now = Utc::now();
    let mut graph = ConversationGraph::new("");

    let tc_id = Uuid::new_v4();
    graph.add_node(Node::ToolCall {
        id: tc_id,
        api_tool_use_id: None,
        arguments: ToolCallArguments::ReadFile {
            path: "x.rs".into(),
        },
        status: ToolCallStatus::Completed,
        parent_message_id: Uuid::new_v4(),
        created_at: now,
        completed_at: Some(now),
    });
    graph.add_node(Node::ToolResult {
        id: Uuid::new_v4(),
        tool_call_id: tc_id,
        content: crate::graph::tool_types::ToolResultContent::text("ok"),
        is_error: false,
        created_at: now,
    });

    let entries: Vec<TaskEntry> = graph
        .nodes_by(is_task_node)
        .into_iter()
        .filter_map(|n| TaskEntry::from_node(n, now))
        .collect();

    assert_eq!(
        entries.len(),
        1,
        "ToolResult should not appear as a task entry"
    );
}
