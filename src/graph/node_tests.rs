use super::*;
use crate::graph::tool_types::{ToolCallArguments, ToolCallStatus, ToolResultContent};
use chrono::Utc;
use uuid::Uuid;

/// Bug: `content()` returns wrong field for message-like variants —
/// e.g., `SystemDirective` returns id instead of content string.
#[test]
fn test_content_message_like_variants() {
    let now = Utc::now();
    let id = Uuid::new_v4();

    let msg = Node::Message {
        id,
        role: Role::User,
        content: "msg_text".to_string(),
        created_at: now,
        model: None,
        input_tokens: None,
        output_tokens: None,
        stop_reason: None,
    };
    assert_eq!(msg.content(), "msg_text");

    let sys = Node::SystemDirective {
        id,
        content: "sys_text".to_string(),
        created_at: now,
    };
    assert_eq!(sys.content(), "sys_text");

    let think = Node::ThinkBlock {
        id,
        content: "thinking...".to_string(),
        parent_message_id: Uuid::new_v4(),
        created_at: now,
    };
    assert_eq!(think.content(), "thinking...");

    let q = Node::Question {
        id,
        content: "q_text".to_string(),
        destination: QuestionDestination::User,
        status: QuestionStatus::Pending,
        requires_approval: false,
        created_at: now,
    };
    assert_eq!(q.content(), "q_text");

    let a = Node::Answer {
        id,
        content: "a_text".to_string(),
        question_id: Uuid::new_v4(),
        created_at: now,
    };
    assert_eq!(a.content(), "a_text");

    let err = Node::ApiError {
        id,
        message: "api_err".to_string(),
        created_at: now,
    };
    assert_eq!(err.content(), "api_err");
}

/// Bug: `content()` returns wrong field for entity variants — e.g.,
/// `WorkItem` returns `description` instead of `title`.
#[test]
fn test_content_entity_variants() {
    let now = Utc::now();
    let id = Uuid::new_v4();

    let wi = Node::WorkItem {
        id,
        kind: WorkItemKind::Plan,
        title: "plan_title".to_string(),
        status: WorkItemStatus::Todo,
        description: None,
        created_at: now,
    };
    assert_eq!(wi.content(), "plan_title");

    let git = Node::GitFile {
        id,
        path: "src/main.rs".to_string(),
        status: GitFileStatus::Modified,
        updated_at: now,
    };
    assert_eq!(git.content(), "src/main.rs");

    let tool = Node::Tool {
        id,
        name: "read_file".to_string(),
        description: "desc".to_string(),
        updated_at: now,
    };
    assert_eq!(tool.content(), "read_file");

    let bg = Node::BackgroundTask {
        id,
        kind: BackgroundTaskKind::GitIndex,
        status: TaskStatus::Running,
        description: "bg_desc".to_string(),
        created_at: now,
        updated_at: now,
    };
    assert_eq!(bg.content(), "bg_desc");

    let tr = Node::ToolResult {
        id,
        tool_call_id: Uuid::new_v4(),
        content: ToolResultContent::text("result_text"),
        is_error: false,
        created_at: now,
    };
    assert_eq!(tr.content(), "result_text");
}

/// Bug: `ToolCall.content()` panics or returns empty instead of
/// `arguments.tool_name()`, breaking display in conversation view.
#[test]
fn test_content_tool_call_returns_tool_name() {
    let node = Node::ToolCall {
        id: Uuid::new_v4(),
        api_tool_use_id: None,
        arguments: ToolCallArguments::ReadFile {
            path: "/tmp/f".to_string(),
        },
        status: ToolCallStatus::Completed,
        parent_message_id: Uuid::new_v4(),
        created_at: Utc::now(),
        completed_at: None,
    };
    assert_eq!(node.content(), "read_file");
}

/// Bug: `input_tokens()` returns `Some` for a non-Message variant,
/// causing incorrect token budget calculations.
#[test]
fn test_input_tokens_none_for_non_message() {
    let msg = Node::Message {
        id: Uuid::new_v4(),
        role: Role::Assistant,
        content: String::new(),
        created_at: Utc::now(),
        model: None,
        input_tokens: Some(42),
        output_tokens: None,
        stop_reason: None,
    };
    assert_eq!(msg.input_tokens(), Some(42));

    let wi = Node::WorkItem {
        id: Uuid::new_v4(),
        kind: WorkItemKind::Task,
        title: String::new(),
        status: WorkItemStatus::Todo,
        description: None,
        created_at: Utc::now(),
    };
    assert_eq!(wi.input_tokens(), None);
}

/// Bug: `output_tokens()` returns `Some` for a non-Message variant.
#[test]
fn test_output_tokens_none_for_non_message() {
    let msg = Node::Message {
        id: Uuid::new_v4(),
        role: Role::Assistant,
        content: String::new(),
        created_at: Utc::now(),
        model: None,
        input_tokens: None,
        output_tokens: Some(99),
        stop_reason: None,
    };
    assert_eq!(msg.output_tokens(), Some(99));

    let err = Node::ApiError {
        id: Uuid::new_v4(),
        message: String::new(),
        created_at: Utc::now(),
    };
    assert_eq!(err.output_tokens(), None);
}

/// Bug: `model()` returns `Some` for a non-Message variant, polluting
/// metadata display with spurious model names.
#[test]
fn test_model_none_for_non_message() {
    let msg = Node::Message {
        id: Uuid::new_v4(),
        role: Role::Assistant,
        content: String::new(),
        created_at: Utc::now(),
        model: Some("claude-3".to_string()),
        input_tokens: None,
        output_tokens: None,
        stop_reason: None,
    };
    assert_eq!(msg.model(), Some("claude-3"));

    let q = Node::Question {
        id: Uuid::new_v4(),
        content: String::new(),
        destination: QuestionDestination::Llm,
        status: QuestionStatus::Pending,
        requires_approval: false,
        created_at: Utc::now(),
    };
    assert_eq!(q.model(), None);
}

/// Bug: `is_truncated()` returns true for `EndTurn`, causing the TUI
/// to show "truncated" on normal responses. Only `MaxTokens` should be true.
#[test]
fn test_stop_reason_and_is_truncated() {
    let truncated = Node::Message {
        id: Uuid::new_v4(),
        role: Role::Assistant,
        content: String::new(),
        created_at: Utc::now(),
        model: None,
        input_tokens: None,
        output_tokens: None,
        stop_reason: Some(StopReason::MaxTokens),
    };
    assert!(truncated.is_truncated());
    assert_eq!(truncated.stop_reason(), Some(StopReason::MaxTokens));

    let normal = Node::Message {
        id: Uuid::new_v4(),
        role: Role::Assistant,
        content: String::new(),
        created_at: Utc::now(),
        model: None,
        input_tokens: None,
        output_tokens: None,
        stop_reason: Some(StopReason::EndTurn),
    };
    assert!(!normal.is_truncated());

    let tool_use = Node::Message {
        id: Uuid::new_v4(),
        role: Role::Assistant,
        content: String::new(),
        created_at: Utc::now(),
        model: None,
        input_tokens: None,
        output_tokens: None,
        stop_reason: Some(StopReason::ToolUse),
    };
    assert!(!tool_use.is_truncated());

    let no_reason = Node::Message {
        id: Uuid::new_v4(),
        role: Role::Assistant,
        content: String::new(),
        created_at: Utc::now(),
        model: None,
        input_tokens: None,
        output_tokens: None,
        stop_reason: None,
    };
    assert!(!no_reason.is_truncated());
}

/// Bug: `created_at()` for `GitFile`/`Tool` returns a nonexistent
/// `created_at` field instead of `updated_at`, panicking at runtime.
#[test]
fn test_created_at_git_file_uses_updated_at() {
    let timestamp = Utc::now();
    let git = Node::GitFile {
        id: Uuid::new_v4(),
        path: String::new(),
        status: GitFileStatus::Tracked,
        updated_at: timestamp,
    };
    assert_eq!(git.created_at(), timestamp);

    let tool = Node::Tool {
        id: Uuid::new_v4(),
        name: String::new(),
        description: String::new(),
        updated_at: timestamp,
    };
    assert_eq!(tool.created_at(), timestamp);
}

/// Bug: `StopReason::from_api()` mismatches a wire string, causing
/// the agent loop to not detect truncation or tool use.
#[test]
fn test_stop_reason_from_api_all_values() {
    assert_eq!(StopReason::from_api("end_turn"), Some(StopReason::EndTurn));
    assert_eq!(
        StopReason::from_api("max_tokens"),
        Some(StopReason::MaxTokens)
    );
    assert_eq!(StopReason::from_api("tool_use"), Some(StopReason::ToolUse));
    assert_eq!(StopReason::from_api("unknown_future_value"), None);
    assert_eq!(StopReason::from_api(""), None);
}

/// Bug: `Role::Display` returns wrong string, breaking API request
/// serialization that uses the display string.
#[test]
fn test_role_display() {
    assert_eq!(format!("{}", Role::User), "user");
    assert_eq!(format!("{}", Role::Assistant), "assistant");
    assert_eq!(format!("{}", Role::System), "system");
}
