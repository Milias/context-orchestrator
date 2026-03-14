use super::*;

use crate::graph::{ConversationGraph, EdgeKind, Node, Role};
use chrono::Utc;
use std::collections::HashSet;
use uuid::Uuid;

/// Helper: create a message node.
fn msg(content: &str) -> Node {
    Node::Message {
        id: Uuid::new_v4(),
        role: Role::User,
        content: content.to_string(),
        created_at: Utc::now(),
        model: None,
        input_tokens: None,
        output_tokens: None,
        stop_reason: None,
    }
}

/// Bug: `gather` returns duplicate node IDs when the same node is reachable via
/// multiple graph paths. In a diamond shape (A → B, A → C, B → D, C → D),
/// node D is reachable from A via both B and C. Without deduplication, D would
/// appear twice in the candidate list, causing double-scoring and inflated
/// context usage. The `seen` set should prevent this.
#[test]
fn diamond_graph_produces_no_duplicate_candidates() {
    let mut graph = ConversationGraph::new("sys");
    let root = graph.branch_leaf("main").unwrap();

    //   root
    //   / \
    //  b   c
    //   \ /
    //    d
    let b = graph.add_reply(root, msg("b")).unwrap();
    let c = graph.add_reply(root, msg("c")).unwrap();

    let d_node = msg("d");
    let d_id = d_node.id();
    graph.nodes.insert(d_id, d_node);
    // d is a child of both b and c via RespondsTo.
    graph.add_edge(d_id, b, EdgeKind::RespondsTo).unwrap();
    graph.add_edge(d_id, c, EdgeKind::RespondsTo).unwrap();

    let candidates = gather(&graph, root);
    let ids: Vec<Uuid> = candidates.iter().map(|c| c.node_id).collect();
    let unique: HashSet<Uuid> = ids.iter().copied().collect();

    assert_eq!(
        ids.len(),
        unique.len(),
        "gather must not return duplicate node IDs; got {} total but only {} unique",
        ids.len(),
        unique.len()
    );
}

/// Bug: `gather` exceeds `MAX_CANDIDATES` (200), causing unbounded candidate
/// lists that slow down the scoring stage. With 300 message nodes in a chain,
/// the output must be capped at 200.
#[test]
fn gather_caps_at_max_candidates() {
    let mut graph = ConversationGraph::new("sys");
    let root = graph.branch_leaf("main").unwrap();

    // Build a long chain: root → m1 → m2 → ... → m300.
    let mut parent = root;
    for i in 0..300 {
        let m = Node::Message {
            id: Uuid::new_v4(),
            role: Role::User,
            content: format!("msg-{i}"),
            created_at: Utc::now(),
            model: None,
            input_tokens: None,
            output_tokens: None,
            stop_reason: None,
        };
        parent = graph.add_reply(parent, m).unwrap();
    }

    let candidates = gather(&graph, root);

    assert!(
        candidates.len() <= MAX_CANDIDATES,
        "gather must cap output at MAX_CANDIDATES ({}), got {}",
        MAX_CANDIDATES,
        candidates.len()
    );
}
