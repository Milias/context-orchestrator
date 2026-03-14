use crate::graph::node::{QuestionDestination, QuestionStatus};
use crate::graph::tool::types::{ToolCallArguments, ToolCallStatus};
use crate::graph::{ConversationGraph, EdgeKind, Node};
use chrono::Utc;
use uuid::Uuid;

/// Bug: `Question` node not created, or `Asks` edge points in wrong direction.
/// The `Asks` edge should go `ToolCall → Question` (provenance: who asked).
#[test]
fn test_ask_creates_question_with_asks_edge() {
    let mut graph = ConversationGraph::new("system");

    let tc_id = Uuid::new_v4();
    graph.add_node(Node::ToolCall {
        id: tc_id,
        api_tool_use_id: None,
        arguments: ToolCallArguments::Ask {
            question: "What JWT library?".to_string(),
            destination: QuestionDestination::User,
            about_node_id: None,
            requires_approval: None,
        },
        status: ToolCallStatus::Running,
        parent_message_id: Uuid::new_v4(),
        created_at: Utc::now(),
        completed_at: None,
    });

    let result = super::apply(&mut graph, tc_id);
    assert!(result.is_some(), "ask tool should produce enriched content");

    // Find the created Question node.
    let questions = graph.open_questions();
    assert_eq!(questions.len(), 1, "exactly one Question should exist");
    let q_id = questions[0].id();

    // Verify Asks edge direction: ToolCall → Question.
    let asks_edges: Vec<_> = graph
        .edges
        .iter()
        .filter(|e| e.kind == EdgeKind::Asks)
        .collect();
    assert_eq!(asks_edges.len(), 1);
    assert_eq!(
        asks_edges[0].from, tc_id,
        "Asks edge should start from ToolCall"
    );
    assert_eq!(asks_edges[0].to, q_id, "Asks edge should point to Question");
}

/// Bug: `About` edge missing when `about_node_id` references an existing node.
#[test]
fn test_ask_with_about_creates_about_edge() {
    let mut graph = ConversationGraph::new("system");

    // Create a WorkItem to reference.
    let wi_id = Uuid::new_v4();
    graph.add_node(Node::WorkItem {
        id: wi_id,
        kind: crate::graph::WorkItemKind::Plan,
        title: "Auth module".to_string(),
        status: crate::graph::WorkItemStatus::Todo,
        description: None,
        created_at: Utc::now(),
    });

    let tc_id = Uuid::new_v4();
    graph.add_node(Node::ToolCall {
        id: tc_id,
        api_tool_use_id: None,
        arguments: ToolCallArguments::Ask {
            question: "Should we refactor this?".to_string(),
            destination: QuestionDestination::Llm,
            about_node_id: Some(wi_id),
            requires_approval: None,
        },
        status: ToolCallStatus::Running,
        parent_message_id: Uuid::new_v4(),
        created_at: Utc::now(),
        completed_at: None,
    });

    super::apply(&mut graph, tc_id);

    let q_id = graph.open_questions()[0].id();

    // Verify About edge: Question → WorkItem.
    let about_edges: Vec<_> = graph
        .edges
        .iter()
        .filter(|e| e.kind == EdgeKind::About)
        .collect();
    assert_eq!(about_edges.len(), 1);
    assert_eq!(
        about_edges[0].from, q_id,
        "About edge should start from Question"
    );
    assert_eq!(
        about_edges[0].to, wi_id,
        "About edge should point to referenced node"
    );
}

/// Bug: crash when `about_node_id` references a non-existent node.
/// The Question should still be created; the About edge is just skipped.
#[test]
fn test_ask_with_invalid_about_skips_edge() {
    let mut graph = ConversationGraph::new("system");

    let tc_id = Uuid::new_v4();
    graph.add_node(Node::ToolCall {
        id: tc_id,
        api_tool_use_id: None,
        arguments: ToolCallArguments::Ask {
            question: "Is this safe?".to_string(),
            destination: QuestionDestination::Auto,
            about_node_id: Some(Uuid::new_v4()), // non-existent
            requires_approval: Some(true),
        },
        status: ToolCallStatus::Running,
        parent_message_id: Uuid::new_v4(),
        created_at: Utc::now(),
        completed_at: None,
    });

    let result = super::apply(&mut graph, tc_id);
    assert!(result.is_some(), "should still return enriched content");

    // Question created despite invalid about_node_id.
    let questions = graph.open_questions();
    assert_eq!(questions.len(), 1);

    // No About edges.
    let about_count = graph
        .edges
        .iter()
        .filter(|e| e.kind == EdgeKind::About)
        .count();
    assert_eq!(about_count, 0, "no About edge for non-existent node");

    // Verify requires_approval is set.
    if let Node::Question {
        requires_approval, ..
    } = questions[0]
    {
        assert!(requires_approval, "requires_approval should be true");
    } else {
        panic!("expected Question node");
    }
}

/// Helper: create a Claimed question in the graph, ready for answering.
fn create_claimed_question(graph: &mut ConversationGraph, content: &str) -> Uuid {
    let q_id = Uuid::new_v4();
    graph.add_node(Node::Question {
        id: q_id,
        content: content.to_string(),
        destination: QuestionDestination::Llm,
        status: QuestionStatus::Claimed,
        requires_approval: false,
        created_at: Utc::now(),
    });
    q_id
}

/// Bug: `apply()` doesn't recognize Answer arguments, answer never created.
/// The agent calls `answer` but the Question stays Claimed indefinitely.
#[test]
fn test_answer_creates_answer_node() {
    let mut graph = ConversationGraph::new("system");
    let q_id = create_claimed_question(&mut graph, "What auth strategy?");

    let tc_id = Uuid::new_v4();
    graph.add_node(Node::ToolCall {
        id: tc_id,
        api_tool_use_id: None,
        arguments: ToolCallArguments::Answer {
            question_id: q_id,
            content: "Use JWT with refresh tokens.".to_string(),
        },
        status: ToolCallStatus::Running,
        parent_message_id: Uuid::new_v4(),
        created_at: Utc::now(),
        completed_at: None,
    });

    let result = super::apply(&mut graph, tc_id);
    assert!(
        result.is_some(),
        "answer tool should produce enriched content"
    );

    let content = result.unwrap();
    assert!(
        content.text_content().contains("Answer created"),
        "should confirm answer creation"
    );

    // Question should now be Answered.
    if let Some(Node::Question { status, .. }) = graph.node(q_id) {
        assert_eq!(*status, QuestionStatus::Answered);
    } else {
        panic!("question node should exist");
    }

    // Verify Answer node and Answers edge exist.
    let answer_edges: Vec<_> = graph
        .edges
        .iter()
        .filter(|e| e.kind == EdgeKind::Answers)
        .collect();
    assert_eq!(answer_edges.len(), 1);
    assert_eq!(
        answer_edges[0].to, q_id,
        "Answers edge should point to Question"
    );
}

/// Bug: answer accepted for Pending question, violating the state machine.
/// Only Claimed questions should be answerable — Pending means no agent owns it.
#[test]
fn test_answer_fails_for_unclaimed_question() {
    let mut graph = ConversationGraph::new("system");

    let q_id = Uuid::new_v4();
    graph.add_node(Node::Question {
        id: q_id,
        content: "Unanswerable question".to_string(),
        destination: QuestionDestination::Llm,
        status: QuestionStatus::Pending,
        requires_approval: false,
        created_at: Utc::now(),
    });

    let tc_id = Uuid::new_v4();
    graph.add_node(Node::ToolCall {
        id: tc_id,
        api_tool_use_id: None,
        arguments: ToolCallArguments::Answer {
            question_id: q_id,
            content: "This should fail.".to_string(),
        },
        status: ToolCallStatus::Running,
        parent_message_id: Uuid::new_v4(),
        created_at: Utc::now(),
        completed_at: None,
    });

    let result = super::apply(&mut graph, tc_id);
    assert!(result.is_some(), "should still return content");

    let content = result.unwrap();
    assert!(
        content.text_content().contains("failed"),
        "should report failure for unclaimed question"
    );

    // Question should remain Pending.
    if let Some(Node::Question { status, .. }) = graph.node(q_id) {
        assert_eq!(*status, QuestionStatus::Pending);
    } else {
        panic!("question node should exist");
    }
}
