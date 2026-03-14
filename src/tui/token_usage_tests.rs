//! Tests for [`AnimatedCounter`] and [`TokenUsage`].

use crate::tui::{AnimatedCounter, TokenUsage};

/// Catches: animation getting stuck (never reaching target), overshooting
/// (exceeding target), or infinite loop. Sets a large target and verifies
/// convergence within a bounded number of ticks.
#[test]
fn tick_converges_to_target() {
    let mut counter = AnimatedCounter {
        current: 0,
        target: 10_000,
    };
    // Ease-out at 25% per tick: worst case ~60 ticks to converge from 0 → 10 000.
    for _ in 0..100 {
        counter.tick();
        assert!(
            counter.current <= counter.target,
            "overshot: {}",
            counter.current
        );
        if counter.current == counter.target {
            break;
        }
    }
    assert_eq!(counter.current, counter.target, "did not converge");
    assert!(!counter.is_animating());
}

/// Catches: counter decrementing or changing state when already at target.
#[test]
fn tick_noop_when_at_target() {
    let mut counter = AnimatedCounter {
        current: 500,
        target: 500,
    };
    assert!(!counter.is_animating());
    counter.tick();
    assert_eq!(counter.current, 500);
    assert_eq!(counter.target, 500);
}

/// Catches: `is_animating` not reflecting child counter states.
#[test]
fn token_usage_animating_reflects_children() {
    let mut usage = TokenUsage::default();
    assert!(!usage.is_animating());

    usage.input.target = 100;
    assert!(usage.is_animating());

    // Tick until input converges.
    for _ in 0..100 {
        usage.tick();
        if !usage.is_animating() {
            break;
        }
    }
    assert!(!usage.is_animating());
    assert_eq!(usage.input.current, 100);
}
