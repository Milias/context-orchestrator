use crate::graph::node::{QuestionDestination, QuestionStatus};
use crate::graph::{ConversationGraph, EdgeKind, Node};
use chrono::Utc;
use uuid::Uuid;

/// Helper: create a Question node in the graph.
fn add_question(graph: &mut ConversationGraph, status: QuestionStatus) -> Uuid {
    let id = Uuid::new_v4();
    graph.add_node(Node::Question {
        id,
        content: "test question".to_string(),
        destination: QuestionDestination::User,
        status,
        requires_approval: false,
        created_at: Utc::now(),
    });
    id
}

/// Bug: invalid transitions accepted (e.g., `Pending` → `Answered` skipping `Claimed`).
/// The state machine must reject transitions that skip required intermediate states.
#[test]
fn test_question_status_valid_transitions() {
    let mut graph = ConversationGraph::new("system");
    let q_id = add_question(&mut graph, QuestionStatus::Pending);

    // Valid: Pending → Claimed
    assert!(graph
        .update_question_status(q_id, QuestionStatus::Claimed)
        .is_ok());

    // Invalid: Claimed → Pending (not a valid transition)
    assert!(graph
        .update_question_status(q_id, QuestionStatus::Pending)
        .is_err());

    // Valid: Claimed → Answered
    assert!(graph
        .update_question_status(q_id, QuestionStatus::Answered)
        .is_ok());

    // Invalid: Answered → anything (terminal state)
    assert!(graph
        .update_question_status(q_id, QuestionStatus::Pending)
        .is_err());
}

/// Bug: `Pending` → `Answered` bypasses claiming. Must be rejected.
#[test]
fn test_question_cannot_skip_claimed() {
    let mut graph = ConversationGraph::new("system");
    let q_id = add_question(&mut graph, QuestionStatus::Pending);

    assert!(graph
        .update_question_status(q_id, QuestionStatus::Answered)
        .is_err());
    assert!(graph
        .update_question_status(q_id, QuestionStatus::PendingApproval)
        .is_err());
}

/// Bug: Question stays `Pending` after `add_answer()`. The answer should
/// transition the question to `Answered` (or `PendingApproval`).
#[test]
fn test_add_answer_transitions_to_answered() {
    let mut graph = ConversationGraph::new("system");
    let q_id = add_question(&mut graph, QuestionStatus::Pending);

    // Must claim first
    graph
        .update_question_status(q_id, QuestionStatus::Claimed)
        .unwrap();

    let answer_id = graph.add_answer(q_id, "JWT is best".to_string()).unwrap();

    // Verify status transitioned
    match graph.node(q_id) {
        Some(Node::Question { status, .. }) => {
            assert_eq!(*status, QuestionStatus::Answered);
        }
        _ => panic!("Expected Question node"),
    }

    // Verify Answers edge exists
    let has_answers_edge = graph
        .edges
        .iter()
        .any(|e| e.from == answer_id && e.to == q_id && e.kind == EdgeKind::Answers);
    assert!(
        has_answers_edge,
        "Answers edge should link Answer → Question"
    );
}

/// Bug: `requires_approval` flag ignored — answer should go to
/// `PendingApproval` instead of `Answered`.
#[test]
fn test_add_answer_with_approval_transitions_to_pending_approval() {
    let mut graph = ConversationGraph::new("system");
    let q_id = Uuid::new_v4();
    graph.add_node(Node::Question {
        id: q_id,
        content: "approve this?".to_string(),
        destination: QuestionDestination::Llm,
        status: QuestionStatus::Pending,
        requires_approval: true,
        created_at: Utc::now(),
    });

    graph
        .update_question_status(q_id, QuestionStatus::Claimed)
        .unwrap();
    graph.add_answer(q_id, "I suggest JWT".to_string()).unwrap();

    match graph.node(q_id) {
        Some(Node::Question { status, .. }) => {
            assert_eq!(*status, QuestionStatus::PendingApproval);
        }
        _ => panic!("Expected Question node"),
    }
}

/// Bug: `add_answer` accepts questions not in `Claimed` state.
#[test]
fn test_add_answer_requires_claimed_state() {
    let mut graph = ConversationGraph::new("system");
    let q_id = add_question(&mut graph, QuestionStatus::Pending);

    // Should fail — question is Pending, not Claimed
    assert!(graph.add_answer(q_id, "answer".to_string()).is_err());
}

/// Bug: two agents both claim the same node (double-execution).
/// Second `try_claim` must return `false`.
#[test]
fn test_try_claim_prevents_double_claim() {
    let mut graph = ConversationGraph::new("system");
    let q_id = add_question(&mut graph, QuestionStatus::Pending);
    let agent_a = Uuid::new_v4();
    let agent_b = Uuid::new_v4();

    assert!(graph.try_claim(q_id, agent_a), "first claim should succeed");
    assert!(
        !graph.try_claim(q_id, agent_b),
        "second claim should be rejected"
    );
    assert!(graph.is_claimed(q_id));
}

/// Bug: stale `ClaimedBy` edges survive restart. `release_all_claims` must
/// clear all of them so crashed agents' work becomes re-claimable.
#[test]
fn test_release_all_claims_clears_edges() {
    let mut graph = ConversationGraph::new("system");
    let q1 = add_question(&mut graph, QuestionStatus::Pending);
    let q2 = add_question(&mut graph, QuestionStatus::Pending);
    let agent = Uuid::new_v4();

    graph.try_claim(q1, agent);
    graph.try_claim(q2, agent);
    assert!(graph.is_claimed(q1));
    assert!(graph.is_claimed(q2));

    graph.release_all_claims();

    assert!(!graph.is_claimed(q1), "claim should be released");
    assert!(!graph.is_claimed(q2), "claim should be released");
}
