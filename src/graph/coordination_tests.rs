use crate::graph::node::{QuestionDestination, QuestionStatus};
use crate::graph::{ConversationGraph, Node, WorkItemKind, WorkItemStatus};
use chrono::Utc;
use uuid::Uuid;

/// Bug: `try_claim` allows double-claiming — two agents both think they own the node.
#[test]
fn test_try_claim_prevents_double_claim() {
    let mut graph = ConversationGraph::new("system");
    let id = Uuid::new_v4();
    graph.add_node(Node::WorkItem {
        id,
        kind: WorkItemKind::Task,
        title: "task".to_string(),
        status: WorkItemStatus::Todo,
        description: None,
        created_at: Utc::now(),
    });

    let agent_a = Uuid::new_v4();
    let agent_b = Uuid::new_v4();
    assert!(graph.try_claim(id, agent_a), "first claim should succeed");
    assert!(
        !graph.try_claim(id, agent_b),
        "second claim should fail — already claimed"
    );
}

/// Bug: `release_claim` does not remove the `ClaimedBy` edge — node
/// stays claimed forever, preventing re-assignment.
#[test]
fn test_release_claim_makes_unclaimed() {
    let mut graph = ConversationGraph::new("system");
    let id = Uuid::new_v4();
    graph.add_node(Node::WorkItem {
        id,
        kind: WorkItemKind::Task,
        title: "task".to_string(),
        status: WorkItemStatus::Todo,
        description: None,
        created_at: Utc::now(),
    });

    let agent = Uuid::new_v4();
    graph.try_claim(id, agent);
    assert!(graph.is_claimed(id));

    graph.release_claim(id);
    assert!(!graph.is_claimed(id), "should be unclaimed after release");
}

/// Bug: `open_questions` includes Answered/TimedOut questions, polluting
/// the context with resolved questions.
#[test]
fn test_open_questions_excludes_terminal_states() {
    let mut graph = ConversationGraph::new("system");

    let pending = Uuid::new_v4();
    graph.add_node(Node::Question {
        id: pending,
        content: "pending".to_string(),
        destination: QuestionDestination::User,
        status: QuestionStatus::Pending,
        requires_approval: false,
        created_at: Utc::now(),
    });

    let answered = Uuid::new_v4();
    graph.add_node(Node::Question {
        id: answered,
        content: "answered".to_string(),
        destination: QuestionDestination::User,
        status: QuestionStatus::Answered,
        requires_approval: false,
        created_at: Utc::now(),
    });

    let timed_out = Uuid::new_v4();
    graph.add_node(Node::Question {
        id: timed_out,
        content: "timed out".to_string(),
        destination: QuestionDestination::User,
        status: QuestionStatus::TimedOut,
        requires_approval: false,
        created_at: Utc::now(),
    });

    let open = graph.open_questions();
    assert_eq!(open.len(), 1, "only Pending question should be open");
    assert_eq!(open[0].id(), pending);
}
