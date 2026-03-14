//! Graph event dispatch: routes events to coordination logic and TUI handler.
//!
//! Coordination reactions (question routing, claim management) happen in the
//! explicit match. Every event also passes through the TUI handler for display
//! updates.

use crate::graph::event::GraphEvent;
use crate::graph::node::{CompletionConfidence, QuestionDestination, QuestionStatus};
use crate::graph::{EdgeKind, Node, Role};
use crate::llm::ChatMessage;
use crate::storage::{TokenDirection, TokenEvent};
use crate::tasks::{AgentPhase, TaskMessage};
use crate::tui;

use chrono::Utc;
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
            } => {
                tracing::debug!("WorkItem {node_id} → {new_status:?}");
                if *new_status == crate::graph::WorkItemStatus::Active {
                    self.spawn_task_agent(*node_id);
                }
            }
            GraphEvent::DependencyAdded { from_id, to_id } => {
                tracing::debug!("Dependency: {from_id} → {to_id}");
            }
            GraphEvent::NodeClaimed { node_id, agent_id } => {
                tracing::debug!("Node {node_id} claimed by {agent_id}");
            }
            GraphEvent::MessageAdded { node_id, role } => {
                tracing::trace!("Message {node_id} ({role:?})");
                if *role == Role::User {
                    self.on_user_message(*node_id);
                }
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
            GraphEvent::CompletionProposed {
                node_id,
                confidence,
            } => {
                self.handle_completion_proposed(*node_id, *confidence);
            }
            _ => {}
        }
        // TUI state — every event passes through the TUI handler.
        tui::event_handler::apply_event(&mut self.tui_state, event);
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
                // In the ephemeral model, LLM-destined questions stay Pending
                // until the next conversational agent spawns and claims them
                // during context building. Future: spawn a dedicated question-
                // response agent here.
                tracing::debug!(
                    "LLM question {question_id} pending — will be claimed by next agent"
                );
            }
            QuestionDestination::Auto => unreachable!("resolved above"),
        }
    }

    /// Handle a task agent proposing completion. Creates a review `Question` for
    /// the user to accept or reject. The question is linked to the work item via
    /// an `About` edge and routed through the standard `QuestionAdded` pipeline.
    fn handle_completion_proposed(&mut self, work_item_id: Uuid, confidence: CompletionConfidence) {
        let title = {
            let g = self.graph.read();
            g.node(work_item_id)
                .map_or_else(|| "(unknown task)".to_string(), |n| n.content().to_string())
        };
        let question_content =
            format!("Task '{title}' completed with {confidence:?} confidence. Accept?");
        let question_id = Uuid::new_v4();
        let question_node = Node::Question {
            id: question_id,
            content: question_content,
            destination: QuestionDestination::User,
            status: QuestionStatus::Pending,
            requires_approval: true,
            created_at: Utc::now(),
        };
        let mut g = self.graph.write();
        g.add_node(question_node);
        let _ = g.add_edge(question_id, work_item_id, EdgeKind::About);
        g.emit(GraphEvent::QuestionAdded {
            node_id: question_id,
            destination: QuestionDestination::User,
        });
    }

    /// React to a new user message: dispatch `/command` triggers and spawn
    /// an ephemeral conversational agent.
    fn on_user_message(&mut self, node_id: Uuid) {
        let content = {
            let g = self.graph.read();
            g.node(node_id).map_or("", Node::content).to_string()
        };
        self.dispatch_user_triggers(&content, node_id);
        self.spawn_conversational_agent();
    }

    /// Parse `/command` triggers from message text and dispatch each through
    /// the tool-call pipeline.
    fn dispatch_user_triggers(&mut self, text: &str, user_msg_id: Uuid) {
        for trigger in crate::tools::parse_triggers(text) {
            let args = crate::tools::parse_user_trigger_args(&trigger.tool_name, &trigger.args);
            let tool_call_id = Uuid::new_v4();
            self.handle_tool_call_dispatched(tool_call_id, user_msg_id, args, None);
        }
    }

    /// Spawn an ephemeral conversational agent for the current user message.
    /// Each user message gets its own agent — no persistent primary loop.
    fn spawn_conversational_agent(&mut self) {
        let agent_id = Uuid::new_v4();
        let (tool_rx, cancel_token) = self.agents.register(agent_id, None);

        // Emit an initial phase event for immediate TUI feedback.
        self.graph.read().emit(GraphEvent::AgentPhaseChanged {
            agent_id,
            phase: AgentPhase::CountingTokens,
        });

        let loop_config = super::agent::AgentLoopConfig {
            graph: Arc::clone(&self.graph),
            provider: Arc::clone(&self.provider),
            model: self.config.anthropic_model.clone(),
            max_tokens: self.config.max_tokens,
            max_context_tokens: self.config.max_context_tokens,
            max_tool_loop_iterations: self.config.max_tool_loop_iterations,
            tools: crate::tool_executor::registered_tool_definitions(),
            agent_id,
            policy: crate::app::context::policies::ContextPolicy::Conversational,
            context_selection: self.config.context_selection,
            context_selector_model: self.config.context_selector_model.clone(),
        };

        super::agent::spawn_agent_loop(loop_config, self.task_tx.clone(), tool_rx, cancel_token);
    }

    /// Spawn an ephemeral task agent for a work item that transitioned to Active.
    /// Creates a git worktree for file isolation and roots the agent's message
    /// chain at the work item node.
    fn spawn_task_agent(&mut self, work_item_id: Uuid) {
        // Guard: concurrency limit.
        if self.agents.active_count() >= self.config.max_concurrent_agents {
            tracing::warn!("Max concurrent agents reached, deferring task {work_item_id}");
            return;
        }
        // Guard: only Task-kind work items (not Plans).
        {
            let g = self.graph.read();
            match g.node(work_item_id) {
                Some(Node::WorkItem {
                    kind: crate::graph::WorkItemKind::Task,
                    ..
                }) => {}
                _ => return,
            }
        }
        // Guard: not already spawned.
        if self.agents.agent_for_work_item(work_item_id).is_some() {
            return;
        }

        let agent_id = Uuid::new_v4();

        // Claim the work item to prevent double-execution.
        {
            let mut g = self.graph.write();
            if !g.try_claim(work_item_id, agent_id) {
                return;
            }
        }

        if !self.create_task_seed_message(work_item_id) {
            return;
        }

        // Create worktree asynchronously, then spawn the agent loop.
        let graph = Arc::clone(&self.graph);
        let provider = Arc::clone(&self.provider);
        let task_tx = self.task_tx.clone();
        let model = self.config.anthropic_model.clone();
        let max_tokens = self.config.max_tokens;
        let max_context_tokens = self.config.max_context_tokens;
        let max_tool_loop_iterations = self.config.max_tool_loop_iterations;
        let context_selection = self.config.context_selection;
        let context_selector_model = self.config.context_selector_model.clone();

        // Register synchronously to hold the agent slot (prevents concurrent spawn race).
        let (tool_rx, cancel_token) = self.agents.register(agent_id, None);
        self.agents.track_work_item(work_item_id, agent_id);

        self.graph.read().emit(GraphEvent::AgentPhaseChanged {
            agent_id,
            phase: AgentPhase::BuildingContext,
        });

        // Spawn async task: create worktree → spawn agent loop.
        tokio::spawn(async move {
            // Create git worktree for file isolation.
            let project_root = std::env::current_dir().unwrap_or_default();
            let working_dir =
                match super::agent::worktree::create_worktree(&project_root, work_item_id).await {
                    Ok(path) => Some(path),
                    Err(e) => {
                        tracing::error!("Failed to create worktree for task {work_item_id}: {e}");
                        None // Fall back to shared filesystem.
                    }
                };

            // Filter tools for task agents.
            let policy =
                crate::app::context::policies::ContextPolicy::TaskExecution { work_item_id };
            let all_tools = crate::tool_executor::registered_tool_definitions();
            let tools = match policy.tool_filter() {
                Some(allowed) => all_tools
                    .into_iter()
                    .filter(|t| allowed.contains(&t.name.as_str()))
                    .collect(),
                None => all_tools,
            };

            let loop_config = super::agent::AgentLoopConfig {
                graph,
                provider,
                model,
                max_tokens,
                max_context_tokens,
                max_tool_loop_iterations,
                tools,
                agent_id,
                policy,
                context_selection,
                context_selector_model,
            };

            if let Some(path) = working_dir {
                let _ = task_tx.send(TaskMessage::WorktreeCreated { agent_id, path });
            }

            super::agent::spawn_agent_loop(loop_config, task_tx, tool_rx, cancel_token);
        });
    }

    /// Create a synthetic User message rooted at the work item to seed the agent's
    /// `RespondsTo` chain. Returns `false` if the work item is missing or the reply
    /// cannot be added (caller should abort agent spawn).
    fn create_task_seed_message(&self, work_item_id: Uuid) -> bool {
        let (title, description) = {
            let g = self.graph.read();
            match g.node(work_item_id) {
                Some(Node::WorkItem {
                    title, description, ..
                }) => (title.clone(), description.clone().unwrap_or_default()),
                _ => return false,
            }
        };
        let synthetic_msg = Node::Message {
            id: Uuid::new_v4(),
            role: crate::graph::Role::User,
            content: format!("Execute this task: {title}\n\n{description}"),
            created_at: Utc::now(),
            model: None,
            input_tokens: None,
            output_tokens: None,
            stop_reason: None,
        };
        let mut g = self.graph.write();
        if let Err(e) = g.add_reply(work_item_id, synthetic_msg) {
            tracing::error!("Failed to create synthetic message for task {work_item_id}: {e}");
            return false;
        }
        true
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
