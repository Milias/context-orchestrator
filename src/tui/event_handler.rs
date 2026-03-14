//! TUI state updates driven by graph events.
//!
//! This is the ONLY module that mutates `TuiState` in response to events.
//! The App calls [`apply_event`] for every graph event; this module decides
//! which fields to update.

use crate::graph::event::GraphEvent;
use crate::graph::node::QuestionStatus;
use crate::graph::StopReason;
use crate::tasks::AgentPhase;

use super::{AgentDisplayState, AgentVisualPhase, ScrollMode, TuiState};
use uuid::Uuid;

/// Apply a graph event to the TUI state. Called once per event by the App.
/// `is_primary` returns whether the given agent ID is the primary agent
/// (controls which agent's events update the display).
pub fn apply_event(state: &mut TuiState, event: &GraphEvent, is_primary: impl Fn(Uuid) -> bool) {
    match event {
        // ── Agent lifecycle ──────────────────────────────────────
        GraphEvent::AgentPhaseChanged { agent_id, phase } => {
            if !is_primary(*agent_id) {
                return;
            }
            state.status_message = Some(phase.to_string());
            if state.agent_display.is_none() {
                state.agent_display = Some(AgentDisplayState::default());
            }
            apply_visual_phase(state, phase);
        }
        GraphEvent::StreamDelta {
            agent_id,
            text,
            is_thinking,
        } => {
            if !is_primary(*agent_id) {
                return;
            }
            if let Some(ref mut d) = state.agent_display {
                d.phase = AgentVisualPhase::Streaming {
                    text: text.clone(),
                    is_thinking: *is_thinking,
                };
            }
            if state.scroll_mode == ScrollMode::Auto {
                state.scroll_offset = u16::MAX;
            }
        }
        GraphEvent::AgentIterationCommitted {
            agent_id,
            assistant_id,
            stop_reason,
        } => {
            if !is_primary(*agent_id) {
                return;
            }
            if *stop_reason == Some(StopReason::MaxTokens) {
                state.error_message =
                    Some("Response truncated — continuing automatically".to_string());
            }
            if let Some(ref mut d) = state.agent_display {
                d.revealed_chars = usize::MAX;
                d.iteration_node_ids.push(*assistant_id);
                if *stop_reason == Some(StopReason::ToolUse) {
                    d.phase = AgentVisualPhase::ExecutingTools;
                }
            }
        }
        GraphEvent::AgentIdle { agent_id } | GraphEvent::AgentFinished { agent_id } => {
            if is_primary(*agent_id) {
                state.agent_display = None;
                state.status_message = None;
            }
        }

        // ── Question lifecycle ───────────────────────────────────
        GraphEvent::QuestionRoutedToUser {
            question_id,
            content,
        } => {
            tracing::debug!("Question {question_id} routed to user");
            state.pending_question_text = Some(content.clone());
            state.status_message = Some(format!("Question: {content}"));
        }
        GraphEvent::QuestionAnswered {
            question_id,
            answer_id,
        } => {
            tracing::debug!("Question {question_id} answered by {answer_id}");
            state.pending_question_text = None;
            state.status_message = None;
        }
        GraphEvent::QuestionStatusChanged { new_status, .. }
            if *new_status == QuestionStatus::TimedOut =>
        {
            state.pending_question_text = None;
            state.status_message = None;
        }

        // ── System events ────────────────────────────────────────
        GraphEvent::ErrorOccurred { message } => {
            state.error_message = Some(message.clone());
        }
        GraphEvent::TokenTotalsUpdated { input, output } => {
            state.token_usage.input.target = *input;
            state.token_usage.output.target = *output;
        }
        _ => {}
    }
}

/// Update the visual phase indicator based on the agent phase.
fn apply_visual_phase(state: &mut TuiState, phase: &AgentPhase) {
    match phase {
        AgentPhase::Receiving => {
            if let Some(ref mut d) = state.agent_display {
                d.phase = AgentVisualPhase::Streaming {
                    text: String::new(),
                    is_thinking: false,
                };
                d.revealed_chars = 0;
            }
        }
        AgentPhase::ExecutingTools { .. } => {
            if let Some(ref mut d) = state.agent_display {
                d.phase = AgentVisualPhase::ExecutingTools;
            }
        }
        AgentPhase::CountingTokens
        | AgentPhase::BuildingContext
        | AgentPhase::Connecting { .. } => {
            if let Some(ref mut d) = state.agent_display {
                if !matches!(d.phase, AgentVisualPhase::Streaming { .. }) {
                    d.phase = AgentVisualPhase::Preparing;
                }
            }
        }
    }
}
