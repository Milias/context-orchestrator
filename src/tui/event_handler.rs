//! TUI state updates driven by graph events.
//!
//! This is the ONLY module that mutates `TuiState` in response to events.
//! The App calls [`apply_event`] for every graph event; this module decides
//! which fields to update.

use crate::graph::event::GraphEvent;
use crate::graph::node::QuestionStatus;
use crate::graph::{Role, StopReason};
use crate::tasks::AgentPhase;

use super::{AgentDisplayState, AgentVisualPhase, ScrollMode, TuiState};

/// Apply a graph event to the TUI state. Called once per event by the App.
/// All agent events route to per-agent display entries keyed by `agent_id`.
pub fn apply_event(state: &mut TuiState, event: &GraphEvent) {
    match event {
        // ── Agent lifecycle ──────────────────────────────────────
        GraphEvent::AgentPhaseChanged { agent_id, phase } => {
            state.status_message = Some(phase.to_string());
            let display = state.agent_displays.entry(*agent_id).or_default();
            apply_visual_phase(display, phase);
        }
        GraphEvent::StreamDelta {
            agent_id,
            text,
            is_thinking,
        } => {
            let display = state.agent_displays.entry(*agent_id).or_default();
            display.phase = AgentVisualPhase::Streaming {
                text: text.clone(),
                is_thinking: *is_thinking,
            };
            state.streaming_agent_id = Some(*agent_id);
            if state.scroll_mode == ScrollMode::Auto {
                state.scroll.snap(u16::MAX);
            }
        }
        GraphEvent::AgentIterationCommitted {
            agent_id,
            assistant_id,
            stop_reason,
        } => {
            if *stop_reason == Some(StopReason::MaxTokens) {
                state.error_message =
                    Some("Response truncated — continuing automatically".to_string());
            }
            if let Some(display) = state.agent_displays.get_mut(agent_id) {
                display.revealed_chars = usize::MAX;
                display.iteration_node_ids.push(*assistant_id);
                if *stop_reason == Some(StopReason::ToolUse) {
                    display.phase = AgentVisualPhase::ExecutingTools;
                }
            }
        }
        GraphEvent::AgentFinished { agent_id } => {
            state.agent_displays.remove(agent_id);
            // Clear streaming pointer if it was this agent.
            if state.streaming_agent_id == Some(*agent_id) {
                state.streaming_agent_id = None;
            }
            // Clear status only when no agents remain.
            if state.agent_displays.is_empty() {
                state.status_message = None;
            }
        }

        // ── User message ────────────────────────────────────────
        GraphEvent::MessageAdded { role, .. } if *role == Role::User => {
            state.scroll_mode = ScrollMode::Auto;
            state.scroll.snap(u16::MAX);
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

/// Update the visual phase indicator on a specific agent display.
fn apply_visual_phase(display: &mut AgentDisplayState, phase: &AgentPhase) {
    match phase {
        AgentPhase::Receiving => {
            display.phase = AgentVisualPhase::Streaming {
                text: String::new(),
                is_thinking: false,
            };
            display.revealed_chars = 0;
        }
        AgentPhase::ExecutingTools { .. } => {
            display.phase = AgentVisualPhase::ExecutingTools;
        }
        AgentPhase::CountingTokens
        | AgentPhase::BuildingContext
        | AgentPhase::Connecting { .. } => {
            if !matches!(display.phase, AgentVisualPhase::Streaming { .. }) {
                display.phase = AgentVisualPhase::Preparing;
            }
        }
    }
}
