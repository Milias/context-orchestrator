//! Deterministic scoring for context candidates.
//!
//! Implements edge-weighted scoring (Strategy K from doc 22) with recency boost.
//! O(V+E), zero API cost, deterministic: same graph state → same scores.

use crate::graph::{ConversationGraph, EdgeKind};

use super::candidates::Candidate;
use chrono::Utc;

/// Selection tier based on score. Determines detail level and budget allocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum SelectionTier {
    /// Must be in context. Full content, always included.
    Essential,
    /// Should be in context if budget allows. Full content.
    Important,
    /// Include if space remains. Rendered as compact summary.
    Supplementary,
}

/// A scored candidate with its selection tier.
#[derive(Debug, Clone)]
pub struct ScoredCandidate {
    pub node_id: Uuid,
    pub score: f64,
    pub tier: SelectionTier,
}

/// Edge weight for scoring. Higher = closer semantic relationship.
fn edge_weight(kind: EdgeKind) -> f64 {
    match kind {
        EdgeKind::RespondsTo => 1.0,
        EdgeKind::Invoked | EdgeKind::Produced => 0.9,
        EdgeKind::SubtaskOf => 0.8,
        EdgeKind::RelevantTo => 0.7,
        EdgeKind::Asks | EdgeKind::Answers | EdgeKind::About => 0.6,
        EdgeKind::DependsOn
        | EdgeKind::Tracks
        | EdgeKind::Indexes
        | EdgeKind::Provides
        | EdgeKind::ThinkingOf
        | EdgeKind::Triggers
        | EdgeKind::Supersedes
        | EdgeKind::ClaimedBy
        | EdgeKind::OccurredDuring => 0.4,
        EdgeKind::SelectedFor | EdgeKind::ConsumedBy => 0.3,
    }
}

/// Score threshold for tier assignment.
const ESSENTIAL_THRESHOLD: f64 = 0.7;
const IMPORTANT_THRESHOLD: f64 = 0.4;
const SUPPLEMENTARY_THRESHOLD: f64 = 0.2;

/// Assign a tier based on the final score.
fn tier_from_score(score: f64) -> Option<SelectionTier> {
    if score >= ESSENTIAL_THRESHOLD {
        Some(SelectionTier::Essential)
    } else if score >= IMPORTANT_THRESHOLD {
        Some(SelectionTier::Important)
    } else if score >= SUPPLEMENTARY_THRESHOLD {
        Some(SelectionTier::Supplementary)
    } else {
        None // Excluded from context.
    }
}

/// Score candidates by shortest weighted distance from the anchor node.
///
/// Uses BFS with edge weights. Each hop reduces the score by the edge weight.
/// A recency boost multiplies the final score: recent nodes score higher.
pub fn score_candidates(
    graph: &ConversationGraph,
    anchor_id: Uuid,
    candidates: &[Candidate],
) -> Vec<ScoredCandidate> {
    // BFS from anchor, accumulating best scores.
    let mut best_score: std::collections::HashMap<Uuid, f64> = std::collections::HashMap::new();
    best_score.insert(anchor_id, 1.0);

    let mut queue = std::collections::VecDeque::new();
    queue.push_back((anchor_id, 1.0_f64));
    let mut visited = std::collections::HashSet::new();
    visited.insert(anchor_id);

    while let Some((current, current_score)) = queue.pop_front() {
        // Explore all edges from/to this node.
        for edge in &graph.edges {
            let (neighbor, weight) = if edge.from == current {
                (edge.to, edge_weight(edge.kind))
            } else if edge.to == current {
                (edge.from, edge_weight(edge.kind))
            } else {
                continue;
            };

            let new_score = current_score * weight;
            let existing = best_score.get(&neighbor).copied().unwrap_or(0.0);
            if new_score > existing {
                best_score.insert(neighbor, new_score);
                if visited.insert(neighbor) {
                    queue.push_back((neighbor, new_score));
                }
            }
        }
    }

    // Apply recency boost and assign tiers.
    let now = Utc::now();
    candidates
        .iter()
        .filter_map(|c| {
            let base_score = best_score.get(&c.node_id).copied().unwrap_or(0.0);
            // Precision loss is acceptable: age in minutes never exceeds ~2.6M
            // for a 5-year conversation, well within f64 mantissa range.
            #[allow(clippy::cast_precision_loss)]
            let age_minutes = (now - c.created_at).num_minutes().max(1) as f64;
            let recency_boost = 1.0 / (1.0 + age_minutes.ln());
            let final_score = base_score * recency_boost;

            tier_from_score(final_score).map(|tier| ScoredCandidate {
                node_id: c.node_id,
                score: final_score,
                tier,
            })
        })
        .collect()
}
