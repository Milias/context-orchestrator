//! Token budget allocation for tiered context selection.
//!
//! Partitions the token budget across tiers (Essential/Important/Supplementary)
//! and selects the highest-scored nodes that fit within each tier's budget.

use super::scoring::{ScoredCandidate, SelectionTier};

/// Budget allocation ratios per tier.
const ESSENTIAL_RATIO: f64 = 0.60;
const IMPORTANT_RATIO: f64 = 0.30;
const SUPPLEMENTARY_RATIO: f64 = 0.10;

/// Estimated tokens per node for budget calculations.
/// This is a rough heuristic — exact token counts require an API call.
/// The `finalize_context()` step handles precise truncation afterwards.
const ESTIMATED_TOKENS_PER_MESSAGE: u32 = 500;
const ESTIMATED_TOKENS_PER_SUPPLEMENTARY: u32 = 50;

/// Result of budget allocation: which candidates to include at what tier.
#[derive(Debug)]
pub struct BudgetAllocation {
    /// Nodes to include at full detail (Essential + Important).
    pub full_detail: Vec<ScoredCandidate>,
    /// Nodes to include as compact summaries in the system prompt.
    pub supplementary: Vec<ScoredCandidate>,
}

/// Allocate candidates into tiers within the token budget.
///
/// Sorts candidates by score within each tier, then includes as many as
/// fit within the tier's token allocation.
pub fn allocate(mut candidates: Vec<ScoredCandidate>, max_context_tokens: u32) -> BudgetAllocation {
    // Truncation and sign loss are acceptable: ratios are positive constants in (0,1)
    // and max_context_tokens fits in u32, so the products are non-negative and bounded.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let essential_budget = (f64::from(max_context_tokens) * ESSENTIAL_RATIO) as u32;
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let important_budget = (f64::from(max_context_tokens) * IMPORTANT_RATIO) as u32;
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let supplementary_budget = (f64::from(max_context_tokens) * SUPPLEMENTARY_RATIO) as u32;

    // Sort by score descending within each tier.
    candidates.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let mut full_detail = Vec::new();
    let mut supplementary = Vec::new();
    let mut essential_used: u32 = 0;
    let mut important_used: u32 = 0;
    let mut supplementary_used: u32 = 0;

    for candidate in candidates {
        match candidate.tier {
            SelectionTier::Essential => {
                if essential_used + ESTIMATED_TOKENS_PER_MESSAGE <= essential_budget {
                    essential_used += ESTIMATED_TOKENS_PER_MESSAGE;
                    full_detail.push(candidate);
                }
            }
            SelectionTier::Important => {
                if important_used + ESTIMATED_TOKENS_PER_MESSAGE <= important_budget {
                    important_used += ESTIMATED_TOKENS_PER_MESSAGE;
                    full_detail.push(candidate);
                }
            }
            SelectionTier::Supplementary => {
                if supplementary_used + ESTIMATED_TOKENS_PER_SUPPLEMENTARY <= supplementary_budget {
                    supplementary_used += ESTIMATED_TOKENS_PER_SUPPLEMENTARY;
                    supplementary.push(candidate);
                }
            }
        }
    }

    BudgetAllocation {
        full_detail,
        supplementary,
    }
}
