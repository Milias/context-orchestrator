//! Event-driven coordination: question routing, self-scheduling, and answer handling.
//!
//! All cross-component communication flows through graph events. Effects
//! only mutate the graph; subscribers here react to those mutations.

use crate::graph::event::GraphEvent;
use crate::graph::node::{QuestionDestination, QuestionStatus};
use crate::graph::Node;

use std::sync::Arc;
use uuid::Uuid;

use super::agent;
use super::App;

impl App {
    /// User answered a pending question. Creates an `Answer` node in the graph.
    pub(super) fn handle_user_answer(&mut self, question_id: Uuid, text: String) {
        let mut g = self.graph.write();
        match g.add_answer(question_id, text) {
            Ok(_answer_id) => {
                self.tui_state.status_message = None;
            }
            Err(e) => {
                self.tui_state.error_message = Some(format!("Answer failed: {e}"));
            }
        }
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
        }
    }

    /// Route a pending question to its destination backend.
    /// Claims the question via `ClaimedBy` edge, then transitions status.
    fn route_question(&mut self, question_id: Uuid, destination: QuestionDestination) {
        let dest = match destination {
            QuestionDestination::Auto => QuestionDestination::User,
            other => other,
        };

        let agent_id = Uuid::new_v4();
        let mut g = self.graph.write();

        if !g.try_claim(question_id, agent_id) {
            return; // Already claimed by another handler.
        }
        let _ = g.update_question_status(question_id, QuestionStatus::Claimed);
        drop(g);

        match dest {
            QuestionDestination::User => {
                self.pending_user_question = Some(question_id);
                let g = self.graph.read();
                let content = g
                    .node(question_id)
                    .map_or("(question)", Node::content)
                    .to_string();
                self.tui_state.status_message = Some(format!("Question: {content}"));
            }
            QuestionDestination::Llm => {
                self.spawn_question_agent(question_id);
            }
            QuestionDestination::Auto => unreachable!("resolved above"),
        }
    }

    /// Spawn an agent loop to answer an LLM-destined question.
    /// Respects `max_concurrent_agents`; if at capacity, the question stays
    /// Claimed and will be picked up when a slot opens via `check_ready_work`.
    fn spawn_question_agent(&mut self, question_id: Uuid) {
        if self.agents.active_count() >= self.config.max_concurrent_agents {
            return;
        }
        let agent_id = Uuid::new_v4();
        let entry_mode = agent::AgentEntryMode::AnswerQuestion { question_id };
        let (tool_rx, cancel_token) = self.agents.register(agent_id, entry_mode.clone());

        let loop_config = agent::AgentLoopConfig {
            graph: Arc::clone(&self.graph),
            provider: Arc::clone(&self.provider),
            model: self.config.anthropic_model.clone(),
            max_tokens: self.config.max_tokens,
            max_context_tokens: self.config.max_context_tokens,
            max_tool_loop_iterations: self.config.max_tool_loop_iterations,
            tools: crate::tool_executor::registered_tool_definitions(),
            entry_mode,
            anchor_id: question_id,
            agent_id,
        };

        agent::spawn_agent_loop(loop_config, self.task_tx.clone(), tool_rx, cancel_token);
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
