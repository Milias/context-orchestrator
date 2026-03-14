//! Event-driven coordination: question routing, self-scheduling, and answer handling.
//!
//! All cross-component communication flows through graph events. Effects
//! only mutate the graph; subscribers here react to those mutations.
//! Agent/system events also flow through the same bus for TUI updates.

use crate::graph::event::GraphEvent;
use crate::graph::node::{QuestionDestination, QuestionStatus};
use crate::graph::Node;
use crate::tasks::AgentPhase;
use crate::tui::{AgentDisplayState, AgentVisualPhase};

use uuid::Uuid;

use super::App;

impl App {
    /// User answered a pending question. Creates an `Answer` node in the graph.
    /// Status/error updates flow through the `EventBus` (via `QuestionAnswered`
    /// or `ErrorOccurred`).
    pub(super) fn handle_user_answer(&mut self, question_id: Uuid, text: String) {
        let mut g = self.graph.write();
        if let Err(e) = g.add_answer(question_id, text) {
            g.emit(GraphEvent::ErrorOccurred {
                message: format!("Answer failed: {e}"),
            });
        }
        // Status clears via QuestionAnswered → handle_tui_event.
    }

    /// React to graph events. Effects only mutate the graph; all cross-component
    /// communication (TUI updates, question routing, scheduling) happens here.
    pub(super) fn handle_graph_event(&mut self, event: &GraphEvent) {
        match event {
            GraphEvent::QuestionAdded {
                node_id,
                destination,
            } => self.route_question(*node_id, *destination),
            GraphEvent::QuestionStatusChanged {
                node_id,
                new_status,
            } => {
                tracing::debug!("Question {node_id} → {new_status:?}");
                self.check_ready_work();
            }
            GraphEvent::QuestionAnswered {
                question_id,
                answer_id,
            } => {
                tracing::debug!("Question {question_id} answered by {answer_id}");
                if self.pending_user_question == Some(*question_id) {
                    self.pending_user_question = None;
                    self.tui_state.status_message = None;
                }
                self.check_ready_work();
            }
            GraphEvent::WorkItemAdded { node_id, kind } => {
                tracing::debug!("WorkItem {node_id} created ({kind:?})");
                self.check_ready_work();
            }
            GraphEvent::WorkItemStatusChanged {
                node_id,
                new_status,
            } => {
                tracing::debug!("WorkItem {node_id} → {new_status:?}");
                self.check_ready_work();
            }
            GraphEvent::DependencyAdded { from_id, to_id } => {
                tracing::debug!("Dependency: {from_id} → {to_id}");
                self.check_ready_work();
            }
            GraphEvent::NodeClaimed { node_id, agent_id } => {
                tracing::debug!("Node {node_id} claimed by {agent_id}");
            }
            GraphEvent::MessageAdded { node_id, role } => {
                tracing::trace!("Message {node_id} ({role:?})");
            }
            GraphEvent::ToolCallCompleted { node_id, is_error } => {
                tracing::trace!("ToolCall {node_id} completed (error={is_error})");
            }
            GraphEvent::GitFilesRefreshed { count } => {
                tracing::trace!("{count} git files refreshed");
            }
            GraphEvent::ToolsRefreshed { count } => {
                tracing::trace!("{count} tools refreshed");
            }
            GraphEvent::BackgroundTaskChanged { node_id, status } => {
                tracing::trace!("BackgroundTask {node_id} → {status:?}");
            }

            // Agent lifecycle and system events → TUI state.
            _ => self.handle_tui_event(event),
        }
    }

    /// Process agent lifecycle and system events that update TUI state.
    /// Extracted from `handle_graph_event` to keep each method focused.
    fn handle_tui_event(&mut self, event: &GraphEvent) {
        match event {
            GraphEvent::AgentPhaseChanged { agent_id, phase } => {
                if !self.agents.is_primary(*agent_id) {
                    return;
                }
                self.tui_state.status_message = Some(phase.to_string());
                if self.tui_state.agent_display.is_none() {
                    self.tui_state.agent_display = Some(AgentDisplayState::default());
                }
                self.update_visual_phase(phase);
            }
            GraphEvent::StreamDelta {
                agent_id,
                text,
                is_thinking,
            } => {
                if !self.agents.is_primary(*agent_id) {
                    return;
                }
                if let Some(ref mut d) = self.tui_state.agent_display {
                    d.phase = AgentVisualPhase::Streaming {
                        text: text.clone(),
                        is_thinking: *is_thinking,
                    };
                }
                if self.tui_state.scroll_mode == crate::tui::ScrollMode::Auto {
                    self.tui_state.scroll_offset = u16::MAX;
                }
            }
            GraphEvent::AgentIterationCommitted {
                agent_id,
                assistant_id,
                stop_reason,
            } => {
                if !self.agents.is_primary(*agent_id) {
                    return;
                }
                if *stop_reason == Some(crate::graph::StopReason::MaxTokens) {
                    self.tui_state.error_message =
                        Some("Response truncated — continuing automatically".to_string());
                }
                if let Some(ref mut d) = self.tui_state.agent_display {
                    d.revealed_chars = usize::MAX;
                    d.iteration_node_ids.push(*assistant_id);
                    if *stop_reason == Some(crate::graph::StopReason::ToolUse) {
                        d.phase = AgentVisualPhase::ExecutingTools;
                    }
                }
            }
            GraphEvent::AgentFinished { agent_id } => {
                if self.agents.is_primary(*agent_id) {
                    self.tui_state.agent_display = None;
                    self.tui_state.status_message = None;
                }
            }
            GraphEvent::ErrorOccurred { message } => {
                self.tui_state.error_message = Some(message.clone());
            }
            GraphEvent::TokenTotalsUpdated { input, output } => {
                self.tui_state.token_usage.input.target = *input;
                self.tui_state.token_usage.output.target = *output;
            }
            // Graph-mutation events are handled in handle_graph_event; not here.
            _ => {}
        }
    }

    /// Update the TUI visual phase indicator for the primary agent.
    fn update_visual_phase(&mut self, phase: &AgentPhase) {
        match phase {
            AgentPhase::Receiving => {
                if let Some(ref mut d) = self.tui_state.agent_display {
                    d.phase = AgentVisualPhase::Streaming {
                        text: String::new(),
                        is_thinking: false,
                    };
                    d.revealed_chars = 0;
                }
            }
            AgentPhase::ExecutingTools { .. } => {
                if let Some(ref mut d) = self.tui_state.agent_display {
                    d.phase = AgentVisualPhase::ExecutingTools;
                }
            }
            AgentPhase::CountingTokens
            | AgentPhase::BuildingContext
            | AgentPhase::Connecting { .. } => {
                if let Some(ref mut d) = self.tui_state.agent_display {
                    if !matches!(d.phase, AgentVisualPhase::Streaming { .. }) {
                        d.phase = AgentVisualPhase::Preparing;
                    }
                }
            }
        }
    }

    /// Route a pending question to its destination backend.
    ///
    /// - **User**: Claims with a fresh UUID (TUI owns the answer), shows prompt.
    /// - **Llm**: Claims with the primary agent's ID so the agent sees the question
    ///   in its next context build and answers via the `answer` tool.
    /// - **Auto**: Resolves to User for now (heuristic deferred).
    fn route_question(&mut self, question_id: Uuid, destination: QuestionDestination) {
        let dest = match destination {
            QuestionDestination::Auto => QuestionDestination::User,
            other => other,
        };

        match dest {
            QuestionDestination::User => {
                // TUI owns the answer — claim with a standalone UUID.
                let claim_id = Uuid::new_v4();
                let mut g = self.graph.write();
                if !g.try_claim(question_id, claim_id) {
                    return;
                }
                if let Err(e) = g.update_question_status(question_id, QuestionStatus::Claimed) {
                    tracing::warn!("Failed to claim user question {question_id}: {e}");
                    return;
                }
                drop(g);
                self.pending_user_question = Some(question_id);
                // This TUI mutation is event-driven: route_question is called
                // from handle_graph_event → QuestionAdded.
                let g = self.graph.read();
                let content = g
                    .node(question_id)
                    .map_or("(question)", Node::content)
                    .to_string();
                self.tui_state.status_message = Some(format!("Question: {content}"));
            }
            QuestionDestination::Llm => {
                // Claim for the primary agent. It will see the question in its
                // context on the next iteration and answer via the `answer` tool.
                let Some(agent_id) = self.agents.primary_agent_id else {
                    // No agent running — leave Pending. check_ready_work() will
                    // route it when an agent starts.
                    return;
                };
                let mut g = self.graph.write();
                if !g.try_claim(question_id, agent_id) {
                    return;
                }
                if let Err(e) = g.update_question_status(question_id, QuestionStatus::Claimed) {
                    tracing::warn!("Failed to claim LLM question {question_id}: {e}");
                }
            }
            QuestionDestination::Auto => unreachable!("resolved above"),
        }
    }

    /// Check for ready work after an agent finishes or a dependency resolves.
    /// Releases stale claims, routes pending questions, and reports ready work items.
    pub(super) fn check_ready_work(&mut self) {
        let g = self.graph.read();

        // Find Claimed questions whose owning agent is no longer active.
        // These are stale claims from finished/crashed agents — release them
        // so the questions return to the routing pool.
        let stale_claims: Vec<Uuid> = g
            .open_questions()
            .iter()
            .filter(|n| {
                matches!(
                    n,
                    Node::Question {
                        status: QuestionStatus::Claimed,
                        ..
                    }
                )
            })
            .filter(|n| !g.is_claimed(n.id())) // ClaimedBy edge was removed but status lingers
            .map(|n| n.id())
            .collect();

        let pending: Vec<_> = g
            .pending_questions()
            .iter()
            .filter_map(|n| match n {
                Node::Question {
                    id, destination, ..
                } => Some((*id, *destination)),
                _ => None,
            })
            .collect();
        let ready_count = g.ready_unclaimed_nodes().len();
        drop(g);

        // Release stale claims — the edge was already removed in the Finished
        // handler, but the Question status may still be Claimed. We can't
        // transition Claimed → Pending (invalid), so release_claim is for
        // edge cleanup. The question won't show up in pending_questions until
        // status changes. For now, log it.
        for q_id in &stale_claims {
            self.graph.write().release_claim(*q_id);
        }

        for (q_id, dest) in pending {
            self.route_question(q_id, dest);
        }

        if ready_count > 0 {
            self.tui_state.status_message = Some(format!("{ready_count} work item(s) ready"));
        }
    }
}
