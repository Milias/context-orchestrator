use super::*;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

/// Bug: `route_tool_result` returns `true` for an unknown `tool_call_id`,
/// causing callers to believe the result was delivered when it was not.
#[test]
fn test_route_unknown_tool_returns_false() {
    let mut reg = AgentRegistry::new();
    assert!(
        !reg.route_tool_result(Uuid::new_v4()),
        "unknown tool_call_id should return false"
    );
}

/// Bug: `remove` does not clean up `tool_call_owner` entries — stale
/// mappings silently route future completions to a dead channel.
#[test]
fn test_remove_cleans_up_tool_call_owner() {
    let mut reg = AgentRegistry::new();
    let agent_id = Uuid::new_v4();
    let (_, _cancel) = reg.register(agent_id);

    let tc_id = Uuid::new_v4();
    reg.track_tool_call(agent_id, tc_id, CancellationToken::new());

    reg.remove(agent_id);

    // The stale tool_call_id must not route to the removed agent.
    assert!(
        !reg.route_tool_result(tc_id),
        "tool result should not route after agent removal"
    );
}

/// Bug: `cancel_all` cancels tokens but leaves agent state — future
/// registrations or lookups see ghost entries.
#[test]
fn test_cancel_all_clears_state_and_cancels_tokens() {
    let mut reg = AgentRegistry::new();
    let id1 = Uuid::new_v4();
    let id2 = Uuid::new_v4();
    let (_, token1) = reg.register(id1);
    let (_, token2) = reg.register(id2);
    reg.primary_agent_id = Some(id1);

    reg.cancel_all();

    assert!(token1.is_cancelled(), "token1 should be cancelled");
    assert!(token2.is_cancelled(), "token2 should be cancelled");
    assert!(!reg.is_primary(id1), "agents map should be empty");
    assert!(reg.primary_agent_id.is_none(), "primary should be cleared");
}

/// Bug: `drain_phases` returns stale IDs or fails to clear the set,
/// causing duplicate phase-completion processing.
#[test]
fn test_drain_phases_returns_all_and_clears() {
    let mut reg = AgentRegistry::new();
    let agent_id = Uuid::new_v4();
    let (_rx, _cancel) = reg.register(agent_id);

    let p1 = Uuid::new_v4();
    let p2 = Uuid::new_v4();
    reg.track_phase(agent_id, p1);
    reg.track_phase(agent_id, p2);

    let drained = reg.drain_phases(agent_id);
    assert_eq!(drained.len(), 2, "should drain both phases");
    assert!(drained.contains(&p1));
    assert!(drained.contains(&p2));

    // Second drain must be empty.
    let second = reg.drain_phases(agent_id);
    assert!(second.is_empty(), "phases should already be drained");
}

/// Bug: `is_primary` returns true after `remove` — TUI routes display
/// updates to a dead agent.
#[test]
fn test_is_primary_false_after_remove() {
    let mut reg = AgentRegistry::new();
    let agent_id = Uuid::new_v4();
    let (_rx, _cancel) = reg.register(agent_id);
    reg.primary_agent_id = Some(agent_id);
    assert!(reg.is_primary(agent_id));

    reg.remove(agent_id);

    assert!(
        !reg.is_primary(agent_id),
        "primary should be cleared after remove"
    );
}

/// Bug: `route_tool_result` for a tool call whose agent was already
/// removed returns `true` or panics. Must return `false`.
#[test]
fn test_route_after_agent_removed_returns_false() {
    let mut reg = AgentRegistry::new();
    let agent_id = Uuid::new_v4();
    let (_rx, _cancel) = reg.register(agent_id);

    let tc_id = Uuid::new_v4();
    reg.track_tool_call(agent_id, tc_id, CancellationToken::new());

    // Remove the agent but the tool_call_owner entry is cleaned by remove().
    reg.remove(agent_id);

    assert!(
        !reg.route_tool_result(tc_id),
        "should return false for tool of removed agent"
    );
}

/// Bug: `complete_phase` does not actually remove the phase ID, so
/// `drain_phases` returns completed phases again.
#[test]
fn test_complete_phase_removes_id() {
    let mut reg = AgentRegistry::new();
    let agent_id = Uuid::new_v4();
    let (_rx, _cancel) = reg.register(agent_id);

    let phase_id = Uuid::new_v4();
    reg.track_phase(agent_id, phase_id);
    reg.complete_phase(agent_id, &phase_id);

    let drained = reg.drain_phases(agent_id);
    assert!(
        !drained.contains(&phase_id),
        "completed phase should not appear in drain"
    );
}

/// Bug: `child_cancel_token` for unknown agent returns a token that
/// is already cancelled. Must return a fresh, uncancelled token.
#[test]
fn test_child_cancel_token_unknown_agent() {
    let reg = AgentRegistry::new();
    let token = reg.child_cancel_token(Uuid::new_v4());
    assert!(
        !token.is_cancelled(),
        "token for unknown agent should not be pre-cancelled"
    );
}
