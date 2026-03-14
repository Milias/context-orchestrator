use crate::graph::node::{QuestionDestination, QuestionStatus};
use crate::graph::{ConversationGraph, EdgeKind, Node};
use chrono::Utc;
use uuid::Uuid;

/// Bug: claimed questions not surfaced in the system prompt section.
/// The agent never sees questions assigned to it, so it never answers them.
#[test]
fn test_qa_section_shows_claimed_questions_for_agent() {
    let mut graph = ConversationGraph::new("system");
    let agent_id = Uuid::new_v4();

    let q_id = Uuid::new_v4();
    graph.add_node(Node::Question {
        id: q_id,
        content: "What authentication strategy?".to_string(),
        destination: QuestionDestination::Llm,
        status: QuestionStatus::Claimed,
        requires_approval: false,
        created_at: Utc::now(),
    });
    let _ = graph.add_edge(q_id, agent_id, EdgeKind::ClaimedBy);

    let section = super::build_qa_section(&graph, agent_id);
    assert!(section.is_some(), "should produce a Q/A section");
    let text = section.unwrap();
    assert!(
        text.contains(&q_id.to_string()),
        "section should contain the question UUID"
    );
    assert!(
        text.contains("What authentication strategy?"),
        "section should contain the question text"
    );
    assert!(
        text.contains("answer"),
        "section should mention the answer tool"
    );
}

/// Bug: agent sees questions claimed by a different agent, causing conflicts.
#[test]
fn test_qa_section_excludes_other_agents_questions() {
    let mut graph = ConversationGraph::new("system");
    let agent_a = Uuid::new_v4();
    let agent_b = Uuid::new_v4();

    let q_id = Uuid::new_v4();
    graph.add_node(Node::Question {
        id: q_id,
        content: "Other agent's question".to_string(),
        destination: QuestionDestination::Llm,
        status: QuestionStatus::Claimed,
        requires_approval: false,
        created_at: Utc::now(),
    });
    let _ = graph.add_edge(q_id, agent_b, EdgeKind::ClaimedBy);

    let section = super::build_qa_section(&graph, agent_a);
    assert!(
        section.is_none(),
        "agent_a should not see agent_b's questions"
    );
}

/// Bug: already-answered questions persist in context, wasting tokens and
/// confusing the agent into re-answering them.
#[test]
fn test_qa_section_excludes_answered_questions() {
    let mut graph = ConversationGraph::new("system");
    let agent_id = Uuid::new_v4();

    let q_id = Uuid::new_v4();
    graph.add_node(Node::Question {
        id: q_id,
        content: "Resolved question".to_string(),
        destination: QuestionDestination::Llm,
        status: QuestionStatus::Answered,
        requires_approval: false,
        created_at: Utc::now(),
    });
    let _ = graph.add_edge(q_id, agent_id, EdgeKind::ClaimedBy);

    let section = super::build_qa_section(&graph, agent_id);
    assert!(
        section.is_none(),
        "answered questions should not appear in context"
    );
}
