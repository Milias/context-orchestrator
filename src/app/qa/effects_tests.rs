use crate::graph::node::QuestionDestination;
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
    let questions = graph.pending_questions();
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

    let q_id = graph.pending_questions()[0].id();

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
    let questions = graph.pending_questions();
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
