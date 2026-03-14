//! Graph event dispatch: routes events to coordination logic and TUI handler.
//!
//! Coordination reactions (question routing, claim management) happen in the
//! explicit match. Every event also passes through the TUI handler for display
//! updates.

use crate::graph::event::GraphEvent;
use crate::graph::node::{QuestionDestination, QuestionStatus};
use crate::graph::{Node, Role};
use crate::llm::ChatMessage;
use crate::storage::{TokenDirection, TokenEvent};
use crate::tasks::TaskMessage;
use crate::tui;

use std::sync::Arc;
use uuid::Uuid;

use super::App;

impl App {
    /// React to graph events: dispatch coordination logic, then update TUI.
    pub(super) fn handle_graph_event(&mut self, event: &GraphEvent) {
        match event {
            GraphEvent::QuestionAdded {
                node_id,
                destination,
            } => self.route_question(*node_id, *destination),
            GraphEvent::QuestionStatusChanged {
                node_id,
                new_status,
            } => tracing::debug!("Question {node_id} → {new_status:?}"),
            GraphEvent::WorkItemAdded { node_id, kind } => {
                tracing::debug!("WorkItem {node_id} created ({kind:?})");
            }
            GraphEvent::WorkItemStatusChanged {
                node_id,
                new_status,
            } => tracing::debug!("WorkItem {node_id} → {new_status:?}"),
            GraphEvent::DependencyAdded { from_id, to_id } => {
                tracing::debug!("Dependency: {from_id} → {to_id}");
            }
            GraphEvent::NodeClaimed { node_id, agent_id } => {
                tracing::debug!("Node {node_id} claimed by {agent_id}");
            }
            GraphEvent::MessageAdded { node_id, role } => {
                tracing::trace!("Message {node_id} ({role:?})");
                self.spawn_token_count(*node_id);
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
            _ => {}
        }
        // TUI state — every event passes through the TUI handler.
        let agents = &self.agents;
        tui::event_handler::apply_event(&mut self.tui_state, event, |id| agents.is_primary(id));
    }

    /// User answered a pending question. Only mutates the graph.
    pub(super) fn handle_user_answer(&mut self, question_id: Uuid, text: String) {
        let mut g = self.graph.write();
        if let Err(e) = g.add_answer(question_id, text) {
            g.emit(GraphEvent::ErrorOccurred {
                message: format!("Answer failed: {e}"),
            });
        }
    }

    /// Route a question to its destination. Only mutates graph + coordination state.
    fn route_question(&mut self, question_id: Uuid, destination: QuestionDestination) {
        let dest = match destination {
            QuestionDestination::Auto => QuestionDestination::User,
            other => other,
        };
        match dest {
            QuestionDestination::User => {
                let claim_id = Uuid::new_v4();
                let mut g = self.graph.write();
                if !g.try_claim(question_id, claim_id) {
                    return;
                }
                if let Err(e) = g.update_question_status(question_id, QuestionStatus::Claimed) {
                    tracing::warn!("Failed to claim user question {question_id}: {e}");
                    return;
                }
                let content = g
                    .node(question_id)
                    .map_or("(question)", Node::content)
                    .to_string();
                g.emit(GraphEvent::QuestionRoutedToUser {
                    question_id,
                    content,
                });
                drop(g);
                self.pending_user_question = Some(question_id);
            }
            QuestionDestination::Llm => {
                let Some(agent_id) = self.agents.primary_agent_id else {
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

    /// Spawn a background task to count tokens for a message node.
    /// Fires on `MessageAdded` — counts tokens and records to analytics.
    fn spawn_token_count(&self, node_id: Uuid) {
        let content = {
            let g = self.graph.read();
            match g.node(node_id) {
                Some(Node::Message { content, role, .. }) => Some((content.clone(), *role)),
                _ => None,
            }
        };
        let Some((text, role)) = content else {
            return;
        };
        let direction = match role {
            Role::User => TokenDirection::Input,
            Role::Assistant => TokenDirection::Output,
            Role::System => return,
        };
        let provider = Arc::clone(&self.provider);
        let model = self.config.anthropic_model.clone();
        let graph = Arc::clone(&self.graph);
        let store = self.token_store.clone();
        let conversation_id = self.metadata.id.clone();
        let tx = self.task_tx.clone();
        tokio::spawn(async move {
            let msg = vec![ChatMessage::text(role, &text)];
            let Ok(count) = provider.count_tokens(&msg, &model, None, &[]).await else {
                return;
            };
            graph.write().set_input_tokens(node_id, count);
            if let Some(store) = store {
                let event = TokenEvent {
                    conversation_id,
                    direction,
                    tokens: count,
                    model: Some(model),
                };
                if let Err(e) = store.record(&event).await {
                    let _ = tx.send(TaskMessage::AnalyticsError(format!("{e}")));
                    return;
                }
                if let Ok(totals) = store.lifetime_totals().await {
                    let _ = tx.send(TaskMessage::TokenTotalsUpdated(totals));
                }
            }
        });
    }
}
