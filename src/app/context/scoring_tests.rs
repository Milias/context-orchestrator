use super::*;

use crate::app::context::candidates;
use crate::graph::{ConversationGraph, EdgeKind, Node, Role};
use chrono::{Duration, Utc};
use uuid::Uuid;

/// Helper: create a message node with a specific creation time.
fn msg_at(created_at: chrono::DateTime<chrono::Utc>) -> Node {
    Node::Message {
        id: Uuid::new_v4(),
        role: Role::User,
        content: "test".to_string(),
        created_at,
        model: None,
        input_tokens: None,
        output_tokens: None,
        stop_reason: None,
    }
}

/// Helper: create a fresh message node (created now).
fn msg_now() -> Node {
    msg_at(Utc::now())
}

/// Bug: `score_candidates` returns Essential tier for nodes far from the anchor
/// (should be lower). A node connected via a low-weight edge like `Indexes`
/// (weight 0.4) must score lower than a node connected via `RespondsTo` (weight
/// 1.0). If scoring treats all edges equally, context windows fill up with
/// peripheral nodes instead of conversation-adjacent ones.
#[test]
fn distant_edge_scores_lower_than_responds_to() {
    let mut graph = ConversationGraph::new("sys");
    let root = graph.branch_leaf("main").unwrap();

    // Direct child via RespondsTo (weight 1.0).
    let direct_child = msg_now();
    let direct_id = graph.add_reply(root, direct_child).unwrap();

    // Node connected via Indexes edge (weight 0.4).
    let indexed_node = msg_now();
    let indexed_id = graph.add_node(indexed_node);
    graph.add_edge(indexed_id, root, EdgeKind::Indexes).unwrap();

    let now = Utc::now();
    let candidates = vec![
        candidates::Candidate {
            node_id: direct_id,
            created_at: now,
        },
        candidates::Candidate {
            node_id: indexed_id,
            created_at: now,
        },
    ];

    let scored = score_candidates(&graph, root, &candidates);

    let direct_score = scored
        .iter()
        .find(|s| s.node_id == direct_id)
        .expect("direct child should be scored");
    let indexed_score = scored
        .iter()
        .find(|s| s.node_id == indexed_id)
        .expect("indexed node should be scored");

    assert!(
        direct_score.score > indexed_score.score,
        "RespondsTo child (score={}) must score higher than Indexes node (score={}): \
         edge weights 1.0 vs 0.4 should produce different base scores",
        direct_score.score,
        indexed_score.score
    );
}

/// Bug: recency boost makes a recent node score higher than an older node at
/// the same graph distance, but both must still be included (not dropped
/// entirely). Two nodes at 1 minute and 10 minutes old, both directly connected
/// to the anchor via RespondsTo, must both appear in the scored output. If the
/// older node is excluded, agents lose recent conversation context.
#[test]
fn old_node_still_scored_not_excluded() {
    let mut graph = ConversationGraph::new("sys");
    let root = graph.branch_leaf("main").unwrap();

    let recent_time = Utc::now() - Duration::minutes(1);
    let old_time = Utc::now() - Duration::minutes(10);

    let recent_node = msg_at(recent_time);
    let recent_id = graph.add_reply(root, recent_node).unwrap();

    let old_node = msg_at(old_time);
    let old_id = graph.add_reply(root, old_node).unwrap();

    let candidates = vec![
        candidates::Candidate {
            node_id: recent_id,
            created_at: recent_time,
        },
        candidates::Candidate {
            node_id: old_id,
            created_at: old_time,
        },
    ];

    let scored = score_candidates(&graph, root, &candidates);

    let recent_scored = scored
        .iter()
        .find(|s| s.node_id == recent_id)
        .expect("recent node should be scored");
    let old_scored = scored
        .iter()
        .find(|s| s.node_id == old_id)
        .expect("10-minute-old node directly connected to anchor must not be excluded");

    assert!(
        recent_scored.score > old_scored.score,
        "recent node (score={}) should score higher than older node (score={})",
        recent_scored.score,
        old_scored.score
    );
}
