//! Candidate gathering for the context scoring pipeline.
//!
//! Heuristic expansion from a trigger node, bounded at `MAX_CANDIDATES`.
//! Produces a flat list of candidate node IDs for the scoring stage.

use crate::graph::{ConversationGraph, EdgeKind, Node, WorkItemStatus};
use uuid::Uuid;

/// Maximum number of candidates to gather. Keeps the scoring stage fast
/// and bounds the meta-LLM prompt size when LLM refinement is enabled.
const MAX_CANDIDATES: usize = 200;

/// A candidate node for context inclusion, with its graph node ID.
#[derive(Debug, Clone)]
pub struct Candidate {
    pub node_id: Uuid,
    /// Timestamp for recency comparison.
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// Gather candidate nodes from the graph by heuristic expansion.
///
/// The gathering strategy depends on whether we're expanding from a
/// conversation branch or a work item chain. Both produce a bounded
/// candidate set for the scoring stage.
pub fn gather(graph: &ConversationGraph, anchor_id: Uuid) -> Vec<Candidate> {
    let mut seen = std::collections::HashSet::new();
    let mut candidates = Vec::new();

    // 1. Walk RespondsTo ancestors from the anchor (conversation chain).
    walk_responds_to_ancestors(graph, anchor_id, &mut seen, &mut candidates);

    // 2. Walk RespondsTo children from the anchor (agent's own chain).
    walk_responds_to_children(graph, anchor_id, &mut seen, &mut candidates);

    // 3. All active WorkItem nodes.
    for node in graph.nodes_by(|n| {
        matches!(
            n,
            Node::WorkItem { status, .. } if *status != WorkItemStatus::Done
        )
    }) {
        try_add(graph, node.id(), &mut seen, &mut candidates);
    }

    // 4. All Question/Answer nodes.
    for node in graph.nodes_by(|n| matches!(n, Node::Question { .. } | Node::Answer { .. })) {
        try_add(graph, node.id(), &mut seen, &mut candidates);
    }

    // 5. GitFile nodes.
    for node in graph.nodes_by(|n| matches!(n, Node::GitFile { .. })) {
        try_add(graph, node.id(), &mut seen, &mut candidates);
    }

    // 6. ApiError nodes.
    for node in graph.nodes_by(|n| matches!(n, Node::ApiError { .. })) {
        try_add(graph, node.id(), &mut seen, &mut candidates);
    }

    // 7. RelevantTo fan-out from the anchor.
    let relevant_ids: Vec<Uuid> = graph
        .sources_by_edge(anchor_id, EdgeKind::RelevantTo)
        .into_iter()
        .chain(
            graph
                .edges
                .iter()
                .filter(|e| e.to == anchor_id && e.kind == EdgeKind::RelevantTo)
                .map(|e| e.from),
        )
        .collect();
    for id in relevant_ids {
        try_add(graph, id, &mut seen, &mut candidates);
    }

    // Hard cap: prune by recency if exceeded.
    if candidates.len() > MAX_CANDIDATES {
        candidates.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        candidates.truncate(MAX_CANDIDATES);
    }

    candidates
}

/// Try to add a node to the candidate set. No-op if already seen.
fn try_add(
    graph: &ConversationGraph,
    node_id: Uuid,
    seen: &mut std::collections::HashSet<Uuid>,
    candidates: &mut Vec<Candidate>,
) {
    if !seen.insert(node_id) {
        return;
    }
    if let Some(node) = graph.node(node_id) {
        candidates.push(Candidate {
            node_id,
            created_at: node.created_at(),
        });
    }
}

/// Walk `RespondsTo` edges backward from a node to gather ancestor context.
fn walk_responds_to_ancestors(
    graph: &ConversationGraph,
    start_id: Uuid,
    seen: &mut std::collections::HashSet<Uuid>,
    candidates: &mut Vec<Candidate>,
) {
    let mut current = start_id;
    loop {
        if seen.insert(current) {
            if let Some(node) = graph.node(current) {
                candidates.push(Candidate {
                    node_id: current,
                    created_at: node.created_at(),
                });
            }
        }
        // Walk to parent via responds_to index.
        match graph.responds_to.get(&current) {
            Some(&parent) => current = parent,
            None => break,
        }
    }
}

/// Walk `RespondsTo` edges forward from a node to gather descendant context.
fn walk_responds_to_children(
    graph: &ConversationGraph,
    start_id: Uuid,
    seen: &mut std::collections::HashSet<Uuid>,
    candidates: &mut Vec<Candidate>,
) {
    let children = graph.reply_children_of(start_id);
    for &child_id in children {
        if seen.insert(child_id) {
            if let Some(node) = graph.node(child_id) {
                candidates.push(Candidate {
                    node_id: child_id,
                    created_at: node.created_at(),
                });
            }
            // Recurse into children.
            walk_responds_to_children(graph, child_id, seen, candidates);
        }
    }
}
