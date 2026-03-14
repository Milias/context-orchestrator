use super::*;
use crate::graph::node::QuestionDestination;
use crate::graph::{Node, Role, WorkItemKind, WorkItemStatus};
use chrono::Utc;
use uuid::Uuid;

/// Bug: `short_model_name` doesn't strip "claude-" prefix, showing
/// redundant "claude-" in every message's metadata bar.
#[test]
fn test_short_model_name_strips_claude_prefix() {
    assert_eq!(short_model_name("claude-3.5-sonnet"), "3.5-sonnet");
    assert_eq!(short_model_name("claude-opus-4"), "opus-4");
    assert_eq!(short_model_name("gpt-4"), "gpt-4");
    assert_eq!(short_model_name("custom-model"), "custom-model");
}

/// Bug: `format_duration` shows "0.0s" for sub-second durations
/// instead of milliseconds.
#[test]
fn test_format_duration_sub_second() {
    let dur = chrono::TimeDelta::milliseconds(500);
    assert_eq!(format_duration(dur), "500ms");
}

/// Bug: `format_duration` shows wrong decimal for seconds. 2500ms
/// should show "2.5s", not "2.50s" or "2s".
#[test]
fn test_format_duration_seconds() {
    let dur = chrono::TimeDelta::milliseconds(2500);
    assert_eq!(format_duration(dur), "2.5s");
}

/// Bug: `format_duration` shows "90.0s" for durations over a minute
/// instead of "1m 30s".
#[test]
fn test_format_duration_minutes() {
    let dur = chrono::TimeDelta::seconds(90);
    assert_eq!(format_duration(dur), "1m 30s");
}

/// Bug: `role_label` returns wrong string for a variant — causes
/// message bubble to show incorrect sender label.
#[test]
fn test_role_label_all_variants() {
    let now = Utc::now();
    let id = Uuid::new_v4();

    let cases: Vec<(Node, &str)> = vec![
        (
            Node::Message {
                id,
                role: Role::User,
                content: String::new(),
                created_at: now,
                model: None,
                input_tokens: None,
                output_tokens: None,
                stop_reason: None,
            },
            "you",
        ),
        (
            Node::Message {
                id,
                role: Role::Assistant,
                content: String::new(),
                created_at: now,
                model: None,
                input_tokens: None,
                output_tokens: None,
                stop_reason: None,
            },
            "assistant",
        ),
        (
            Node::SystemDirective {
                id,
                content: String::new(),
                created_at: now,
            },
            "system",
        ),
        (
            Node::WorkItem {
                id,
                kind: WorkItemKind::Plan,
                title: String::new(),
                status: WorkItemStatus::Todo,
                description: None,
                completion_confidence: None,
                created_at: now,
            },
            "task",
        ),
        (
            Node::Question {
                id,
                content: String::new(),
                destination: QuestionDestination::User,
                status: crate::graph::node::QuestionStatus::Pending,
                requires_approval: false,
                created_at: now,
            },
            "question",
        ),
        (
            Node::ApiError {
                id,
                message: String::new(),
                created_at: now,
            },
            "error",
        ),
    ];

    for (node, expected) in &cases {
        assert_eq!(
            role_label(node),
            *expected,
            "wrong label for {:?}",
            std::mem::discriminant(node)
        );
    }
}

/// Bug: user and assistant colors swapped — user messages appear
/// green (assistant color) and vice versa.
#[test]
fn test_role_color_user_vs_assistant() {
    let now = Utc::now();
    let id = Uuid::new_v4();

    let user = Node::Message {
        id,
        role: Role::User,
        content: String::new(),
        created_at: now,
        model: None,
        input_tokens: None,
        output_tokens: None,
        stop_reason: None,
    };
    assert_eq!(role_color(&user), Color::Cyan);

    let asst = Node::Message {
        id,
        role: Role::Assistant,
        content: String::new(),
        created_at: now,
        model: None,
        input_tokens: None,
        output_tokens: None,
        stop_reason: None,
    };
    assert_eq!(role_color(&asst), Color::Green);

    let err = Node::ApiError {
        id,
        message: String::new(),
        created_at: now,
    };
    assert_eq!(role_color(&err), Color::Red);
}

/// Bug: token format string broken — metadata shows "None/10out"
/// instead of "5in/10out" for assistant messages with both token counts.
#[test]
fn test_metadata_string_with_tokens() {
    let node = Node::Message {
        id: Uuid::new_v4(),
        role: Role::Assistant,
        content: String::new(),
        created_at: Utc::now(),
        model: Some("claude-3.5-sonnet".to_string()),
        input_tokens: Some(100),
        output_tokens: Some(50),
        stop_reason: None,
    };
    let meta = metadata_string(&node, None);
    assert!(
        meta.contains("100in/50out"),
        "should contain token counts: {meta}"
    );
    assert!(
        meta.contains("3.5-sonnet"),
        "should contain shortened model name: {meta}"
    );
}
