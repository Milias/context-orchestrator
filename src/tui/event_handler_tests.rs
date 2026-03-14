use crate::graph::event::GraphEvent;
use crate::graph::node::QuestionStatus;
use crate::graph::Role;
use crate::tui::{AgentVisualPhase, ScrollMode, TuiState};
use uuid::Uuid;

/// Bug: stale `streaming_agent_id` after agent finishes — TUI tries to
/// read display state for a dead agent, causing a `HashMap` miss.
#[test]
fn agent_finished_clears_streaming_id() {
    let mut state = TuiState::new();
    let agent_id = Uuid::new_v4();
    state.streaming_agent_id = Some(agent_id);
    state.agent_displays.entry(agent_id).or_default();

    super::apply_event(&mut state, &GraphEvent::AgentFinished { agent_id });

    assert!(state.streaming_agent_id.is_none());
    assert!(!state.agent_displays.contains_key(&agent_id));
}

/// Bug: `status_message` cleared while other agents still running — user
/// thinks the system is idle when a second agent is active.
#[test]
fn agent_finished_preserves_status_when_others_active() {
    let mut state = TuiState::new();
    let a1 = Uuid::new_v4();
    let a2 = Uuid::new_v4();
    state.agent_displays.entry(a1).or_default();
    state.agent_displays.entry(a2).or_default();
    state.status_message = Some("Working...".to_string());

    super::apply_event(&mut state, &GraphEvent::AgentFinished { agent_id: a1 });

    assert!(
        state.status_message.is_some(),
        "status should persist while agent a2 is still running"
    );
}

/// Bug: agent display stuck in `Preparing` phase during streaming — user
/// sees no text output even though the LLM is generating.
#[test]
fn stream_delta_sets_streaming_phase() {
    let mut state = TuiState::new();
    let agent_id = Uuid::new_v4();

    super::apply_event(
        &mut state,
        &GraphEvent::StreamDelta {
            agent_id,
            text: "Hello world".to_string(),
            is_thinking: false,
        },
    );

    let display = &state.agent_displays[&agent_id];
    assert!(
        matches!(&display.phase, AgentVisualPhase::Streaming { text, .. } if text == "Hello world"),
        "phase should be Streaming with the delta text"
    );
    assert_eq!(state.streaming_agent_id, Some(agent_id));
}

/// Bug: `QuestionRoutedToUser` does not set `pending_question_text`, so the
/// input box doesn't enter answer mode and the user can't respond.
#[test]
fn question_routed_sets_pending_text() {
    let mut state = TuiState::new();

    super::apply_event(
        &mut state,
        &GraphEvent::QuestionRoutedToUser {
            question_id: Uuid::new_v4(),
            content: "Accept task completion?".to_string(),
        },
    );

    assert_eq!(
        state.pending_question_text.as_deref(),
        Some("Accept task completion?")
    );
}

/// Bug: `QuestionAnswered` does not clear `pending_question_text`, leaving
/// the input box permanently stuck in answer mode.
#[test]
fn question_answered_clears_pending_text() {
    let mut state = TuiState::new();
    state.pending_question_text = Some("old question".to_string());

    super::apply_event(
        &mut state,
        &GraphEvent::QuestionAnswered {
            question_id: Uuid::new_v4(),
            answer_id: Uuid::new_v4(),
        },
    );

    assert!(state.pending_question_text.is_none());
}

/// Bug: timed-out question leaves ghost prompt — user sees an unanswerable
/// question in the input box.
#[test]
fn question_timed_out_clears_pending_text() {
    let mut state = TuiState::new();
    state.pending_question_text = Some("stale question".to_string());

    super::apply_event(
        &mut state,
        &GraphEvent::QuestionStatusChanged {
            node_id: Uuid::new_v4(),
            new_status: QuestionStatus::TimedOut,
        },
    );

    assert!(state.pending_question_text.is_none());
}

/// Bug: scroll stays in Manual mode after user sends a message — new
/// content is not visible because conversation doesn't auto-scroll.
#[test]
fn message_added_user_resets_auto_scroll() {
    let mut state = TuiState::new();
    state.scroll_mode = ScrollMode::Manual;

    super::apply_event(
        &mut state,
        &GraphEvent::MessageAdded {
            node_id: Uuid::new_v4(),
            role: Role::User,
        },
    );

    assert_eq!(state.scroll_mode, ScrollMode::Auto);
}

/// Bug: `TokenTotalsUpdated` doesn't update animation targets — token
/// counter display shows stale numbers that never converge to reality.
#[test]
fn token_totals_updated_sets_targets() {
    let mut state = TuiState::new();

    super::apply_event(
        &mut state,
        &GraphEvent::TokenTotalsUpdated {
            input: 50_000,
            output: 25_000,
        },
    );

    assert_eq!(state.token_usage.input.target, 50_000);
    assert_eq!(state.token_usage.output.target, 25_000);
}
