use crate::tui::{AgentDisplayState, AgentVisualPhase};

/// Catches step-size miscalculation that would leave text permanently
/// unrevealed (e.g. off-by-one in the division or min/max logic).
#[test]
fn advance_reveal_catches_up_within_expected_ticks() {
    let mut display = AgentDisplayState {
        phase: AgentVisualPhase::Streaming {
            text: "a".repeat(100),
            is_thinking: false,
        },
        revealed_chars: 0,
        ..AgentDisplayState::default()
    };

    for _ in 0..10 {
        display.advance_reveal(100);
    }
    assert!(
        display.revealed_chars >= 100,
        "should have caught up, got {}",
        display.revealed_chars
    );
}

/// Catches over-advancing past the total char count, which would cause
/// out-of-bounds slicing in the rendering path.
#[test]
fn advance_reveal_noop_when_caught_up() {
    let mut display = AgentDisplayState {
        phase: AgentVisualPhase::Streaming {
            text: "hello".to_string(),
            is_thinking: false,
        },
        revealed_chars: 5,
        ..AgentDisplayState::default()
    };

    display.advance_reveal(5);
    assert_eq!(display.revealed_chars, 5);
}

/// Catches artificial delay during normal streaming. When pending chars are
/// within the burst threshold, they must be revealed instantly (one tick).
#[test]
fn advance_reveal_instant_for_small_pending() {
    let mut display = AgentDisplayState {
        phase: AgentVisualPhase::Streaming {
            text: String::new(),
            is_thinking: false,
        },
        revealed_chars: 90,
        ..AgentDisplayState::default()
    };

    // 10 chars pending — well under the burst threshold of 15
    display.advance_reveal(100);
    assert_eq!(
        display.revealed_chars, 100,
        "small pending should be revealed in a single tick"
    );
}
