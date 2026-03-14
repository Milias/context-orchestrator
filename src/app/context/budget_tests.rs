use super::*;

use crate::app::context::scoring::{ScoredCandidate, SelectionTier};
use uuid::Uuid;

/// Bug: `allocate` exceeds `max_context_tokens` by not enforcing tier budgets.
/// If 100 Essential candidates each estimated at 500 tokens are submitted
/// with a 10,000-token budget, the Essential tier (60% = 6,000 tokens) should
/// fit at most 12 candidates. If the budget is not enforced, all 100 would be
/// included, blowing the context window and causing API rejection.
#[test]
fn essential_tier_respects_budget_cap() {
    let max_tokens: u32 = 10_000;
    // Essential budget = 60% of 10,000 = 6,000. At 500 tokens/message, max 12 nodes.
    // Truncation is intentional: we want the integer floor of the division.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let essential_budget_nodes = ((f64::from(max_tokens) * 0.60) / 500.0) as usize;

    let candidates: Vec<ScoredCandidate> = (0..100)
        .map(|i| ScoredCandidate {
            node_id: Uuid::new_v4(),
            // All high scores to ensure they stay Essential.
            score: 0.9 - (f64::from(i) * 0.001),
            tier: SelectionTier::Essential,
        })
        .collect();

    let allocation = allocate(candidates, max_tokens);

    assert!(
        allocation.full_detail.len() <= essential_budget_nodes,
        "Essential tier should include at most {} nodes (60% of {} tokens / 500), got {}",
        essential_budget_nodes,
        max_tokens,
        allocation.full_detail.len()
    );
}

/// Bug: Supplementary nodes are dropped entirely instead of being allocated
/// their 10% budget share. If Supplementary candidates exist but the allocator
/// only processes Essential/Important, those nodes never appear in context,
/// losing potentially useful summaries. Verify that supplementary tier gets nodes.
#[test]
fn supplementary_tier_gets_nodes() {
    let max_tokens: u32 = 10_000;
    // Supplementary budget = 10% of 10,000 = 1,000. At 50 tokens/supplementary, max 20 nodes.

    let mut candidates = Vec::new();
    // A few Essential to partially fill that tier.
    for i in 0..3 {
        candidates.push(ScoredCandidate {
            node_id: Uuid::new_v4(),
            score: 0.9 - (f64::from(i) * 0.01),
            tier: SelectionTier::Essential,
        });
    }
    // Many Supplementary candidates.
    for i in 0..30 {
        candidates.push(ScoredCandidate {
            node_id: Uuid::new_v4(),
            score: 0.25 - (f64::from(i) * 0.001),
            tier: SelectionTier::Supplementary,
        });
    }

    let allocation = allocate(candidates, max_tokens);

    assert!(
        !allocation.supplementary.is_empty(),
        "Supplementary tier should contain nodes when candidates exist and budget allows"
    );
    // Supplementary budget = 1,000 tokens / 50 per node = 20 max.
    assert!(
        allocation.supplementary.len() <= 20,
        "Supplementary tier should not exceed its budget, got {} nodes",
        allocation.supplementary.len()
    );
    assert_eq!(
        allocation.full_detail.len(),
        3,
        "Essential candidates should all fit within the Essential budget"
    );
}
