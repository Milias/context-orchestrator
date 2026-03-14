use super::AgentPhase;

/// Bug: wrong status string breaks TUI status bar display.
#[test]
fn test_display_counting_tokens() {
    assert_eq!(
        format!("{}", AgentPhase::CountingTokens),
        "Counting tokens..."
    );
}

/// Bug: wrong status string for building context phase.
#[test]
fn test_display_building_context() {
    assert_eq!(
        format!("{}", AgentPhase::BuildingContext),
        "Building context..."
    );
}

/// Bug: first connection attempt shows retry count "(1/3)" when it
/// should display plain "Connecting..." (no retry indicator on first try).
#[test]
fn test_display_connecting_first_attempt() {
    let phase = AgentPhase::Connecting { attempt: 1, max: 3 };
    assert_eq!(format!("{phase}"), "Connecting...");
}

/// Bug: retry count not shown on subsequent attempts — user cannot
/// distinguish a slow first connect from a retry.
#[test]
fn test_display_connecting_retry() {
    let phase = AgentPhase::Connecting { attempt: 2, max: 3 };
    assert_eq!(format!("{phase}"), "Connecting (2/3)...");
}

/// Bug: `Receiving` phase shows wrong or empty string.
#[test]
fn test_display_receiving() {
    assert_eq!(format!("{}", AgentPhase::Receiving), "Receiving...");
}

/// Bug: tool execution count display broken — shows wrong count
/// or missing pluralization.
#[test]
fn test_display_executing_tools() {
    let phase = AgentPhase::ExecutingTools { count: 3 };
    assert_eq!(format!("{phase}"), "Executing 3 tool call(s)...");
}
