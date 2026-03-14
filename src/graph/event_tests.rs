use crate::graph::event::GraphEvent;
use crate::graph::node::{QuestionDestination, QuestionStatus};
use crate::graph::{ConversationGraph, Node, Role};
use chrono::Utc;
use uuid::Uuid;

/// Bug: `add_message` mutation doesn't emit `MessageAdded` event.
/// Subscribers (TUI, agents) would miss new messages.
#[test]
fn test_add_message_emits_message_added() {
    let mut graph = ConversationGraph::new("system");
    let mut rx = graph.init_event_bus();
    let root = graph.branch_leaf("main").unwrap();

    let msg_id = Uuid::new_v4();
    graph
        .add_message(
            root,
            Node::Message {
                id: msg_id,
                role: Role::User,
                content: "hello".to_string(),
                created_at: Utc::now(),
                model: None,
                input_tokens: None,
                output_tokens: None,
                stop_reason: None,
            },
        )
        .unwrap();

    let event = rx.try_recv().expect("should receive MessageAdded event");
    match event {
        GraphEvent::MessageAdded { node_id, role } => {
            assert_eq!(node_id, msg_id);
            assert_eq!(role, Role::User);
        }
        other => panic!("Expected MessageAdded, got {other:?}"),
    }
}

/// Bug: `try_claim` doesn't notify subscribers. Agents can't react to claims.
#[test]
fn test_claim_emits_node_claimed() {
    let mut graph = ConversationGraph::new("system");
    let mut rx = graph.init_event_bus();

    let q_id = Uuid::new_v4();
    graph.add_node(Node::Question {
        id: q_id,
        content: "test".to_string(),
        destination: QuestionDestination::User,
        status: QuestionStatus::Pending,
        requires_approval: false,
        created_at: Utc::now(),
    });

    let agent_id = Uuid::new_v4();
    graph.try_claim(q_id, agent_id);

    let event = rx.try_recv().expect("should receive NodeClaimed event");
    match event {
        GraphEvent::NodeClaimed {
            node_id,
            agent_id: aid,
        } => {
            assert_eq!(node_id, q_id);
            assert_eq!(aid, agent_id);
        }
        other => panic!("Expected NodeClaimed, got {other:?}"),
    }
}

/// Bug: `update_question_status` doesn't emit events, unlike
/// `update_work_item_status` which emits `WorkItemStatusChanged`.
#[test]
fn test_question_status_change_emits_event() {
    let mut graph = ConversationGraph::new("system");
    let mut rx = graph.init_event_bus();

    let q_id = Uuid::new_v4();
    graph.add_node(Node::Question {
        id: q_id,
        content: "test".to_string(),
        destination: QuestionDestination::Llm,
        status: QuestionStatus::Pending,
        requires_approval: false,
        created_at: Utc::now(),
    });

    graph
        .update_question_status(q_id, QuestionStatus::Claimed)
        .unwrap();

    let event = rx
        .try_recv()
        .expect("should receive QuestionStatusChanged event");
    match event {
        GraphEvent::QuestionStatusChanged {
            node_id,
            new_status,
        } => {
            assert_eq!(node_id, q_id);
            assert_eq!(new_status, QuestionStatus::Claimed);
        }
        other => panic!("Expected QuestionStatusChanged, got {other:?}"),
    }
}

/// Bug: panic when `event_bus` is `None` (tests, deserialization).
/// Mutations must work without a bus.
#[test]
fn test_no_panic_when_bus_absent() {
    let mut graph = ConversationGraph::new("system");
    // Do NOT call init_event_bus — bus stays None.
    let root = graph.branch_leaf("main").unwrap();

    // All mutations should succeed without panicking.
    let msg_id = Uuid::new_v4();
    graph
        .add_message(
            root,
            Node::Message {
                id: msg_id,
                role: Role::Assistant,
                content: "hi".to_string(),
                created_at: Utc::now(),
                model: None,
                input_tokens: None,
                output_tokens: None,
                stop_reason: None,
            },
        )
        .unwrap();

    let q_id = Uuid::new_v4();
    graph.add_node(Node::Question {
        id: q_id,
        content: "q".to_string(),
        destination: QuestionDestination::Auto,
        status: QuestionStatus::Pending,
        requires_approval: false,
        created_at: Utc::now(),
    });
    graph.try_claim(q_id, Uuid::new_v4());
    // No panic = pass.
}
