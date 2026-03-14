mod agent;
mod context;
mod event_dispatch;
mod plan;
mod qa;
mod task_handler;
mod think_splitter;

use crate::config::AppConfig;
use crate::graph::event::GraphEvent;
use crate::graph::node::QuestionStatus;
use crate::graph::{ConversationGraph, Node, Role};
use crate::llm::LlmProvider;
use crate::persistence::{self, ConversationMetadata};
use crate::storage::TokenStore;
use crate::tasks::AgentPhase;
use crate::tasks::{self, TaskMessage};
use crate::tui::input::{self, Action};
use crate::tui::ui;
use crate::tui::{self, AnimatedCounter, TuiState};

use chrono::Utc;
use crossterm::event::{Event, EventStream, KeyEventKind};
use futures::StreamExt;
use parking_lot::RwLock;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use uuid::Uuid;

/// Shared graph type — single source of truth for the conversation graph.
/// Main loop and agent loop both read/write through this. Brief lock holds only.
pub(super) type SharedGraph = Arc<RwLock<ConversationGraph>>;

pub struct App {
    config: AppConfig,
    graph: SharedGraph,
    metadata: ConversationMetadata,
    provider: Arc<dyn LlmProvider>,
    tui_state: TuiState,
    task_rx: mpsc::UnboundedReceiver<TaskMessage>,
    task_tx: mpsc::UnboundedSender<TaskMessage>,
    /// Registry of all active agent loops. Owns routing, cancellation, phase tracking.
    agents: agent::AgentRegistry,
    /// Async analytics store for persistent token tracking.
    token_store: Option<TokenStore>,
    /// Receiver for graph events broadcast by the `EventBus`.
    event_rx: Option<tokio::sync::broadcast::Receiver<GraphEvent>>,
    /// Question currently shown to the user for answering. `None` if no
    /// user-destined question is pending.
    pending_user_question: Option<Uuid>,
}

impl App {
    pub fn new(
        config: AppConfig,
        graph: ConversationGraph,
        metadata: ConversationMetadata,
        provider: Arc<dyn LlmProvider>,
        token_store: Option<TokenStore>,
    ) -> Self {
        let (task_tx, task_rx) = mpsc::unbounded_channel();
        Self {
            config,
            graph: Arc::new(RwLock::new(graph)),
            metadata,
            provider,
            tui_state: TuiState::new(),
            task_rx,
            task_tx,
            agents: agent::AgentRegistry::new(),
            token_store,
            event_rx: None,
            pending_user_question: None,
        }
    }

    /// Initialize graph state, event bus, and background tasks.
    fn startup(&mut self) {
        let mut g = self.graph.write();
        g.expire_stale_tasks();
        g.release_all_claims();
        self.event_rx = Some(g.init_event_bus());
        drop(g);

        tasks::spawn_git_watcher(self.task_tx.clone());
        tasks::spawn_tool_discovery(self.task_tx.clone());
        tasks::spawn_context_summarization(self.task_tx.clone());
    }

    /// Cancel agent, stop tasks, save graph.
    fn shutdown(&mut self) {
        self.agents.cancel_all();
        self.graph.write().stop_running_tasks();
        let _ = self.save();
    }

    pub async fn run(mut self) -> anyhow::Result<()> {
        self.seed_token_usage().await;

        let mut terminal = tui::setup_terminal()?;
        let mut event_stream = EventStream::new();
        let mut spinner_interval = tokio::time::interval(Duration::from_millis(80));
        spinner_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut sigterm =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;

        self.startup();

        {
            let g = self.graph.read();
            terminal.draw(|frame| ui::draw(frame, &g, &mut self.tui_state))?;
        }

        loop {
            if self.tui_state.should_quit {
                break;
            }

            let agent_active = self.tui_state.agent_display.is_some();
            let animating = self.tui_state.token_usage.is_animating();

            tokio::select! {
                maybe_event = event_stream.next() => {
                    if let Some(Ok(Event::Key(key))) = maybe_event {
                        if key.kind != KeyEventKind::Press { continue; }
                        let action = {
                            let g = self.graph.read();
                            input::handle_key_event(key, &mut self.tui_state, &g)
                        };
                        let page = terminal.size().map(|s| s.height / 2).unwrap_or(20);
                        self.handle_action(action, agent_active, page)?;
                    }
                }
                Some(task_msg) = self.task_rx.recv() => {
                    self.handle_task_message(task_msg);
                }
                // Graph event subscriber — decoupled reactions to graph mutations.
                result = async {
                    if let Some(ref mut rx) = self.event_rx {
                        rx.recv().await
                    } else {
                        std::future::pending().await
                    }
                } => {
                    if let Ok(ref event) = result {
                        self.handle_graph_event(event);
                    }
                }
                _ = spinner_interval.tick(), if agent_active || animating => {
                    if let Some(ref mut display) = self.tui_state.agent_display {
                        display.spinner_tick = display.spinner_tick.wrapping_add(1);
                        let total = match &display.phase {
                            crate::tui::AgentVisualPhase::Streaming { text, .. } => {
                                text.chars().count()
                            }
                            _ => 0,
                        };
                        display.advance_reveal(total);
                    }
                    self.tui_state.token_usage.tick();
                }
                _ = sigterm.recv() => {
                    self.shutdown();
                    break;
                }
            }

            // Process any graph events queued during this iteration before drawing.
            self.drain_pending_events();

            {
                let g = self.graph.read();
                terminal.draw(|frame| ui::draw(frame, &g, &mut self.tui_state))?;
            }
        }

        tui::restore_terminal(terminal)?;
        Ok(())
    }

    /// Dispatch a user action from the input handler.
    fn handle_action(
        &mut self,
        action: Action,
        agent_active: bool,
        page_size: u16,
    ) -> anyhow::Result<()> {
        match action {
            Action::Quit => {
                self.shutdown();
                self.tui_state.should_quit = true;
            }
            Action::SendMessage(text) => {
                // If a user question is pending, treat the message as an answer.
                if let Some(q_id) = self.pending_user_question.take() {
                    self.handle_user_answer(q_id, text);
                } else if agent_active {
                    self.tui_state.input_text = text;
                    self.tui_state.input_cursor = self.tui_state.input_text.chars().count();
                } else {
                    self.handle_send_message(text)?;
                }
            }
            Action::ScrollUp | Action::ScrollDown | Action::PageUp | Action::PageDown => {
                self.handle_scroll(&action, page_size);
            }
            Action::DismissQuestion => {
                if let Some(q_id) = self.pending_user_question.take() {
                    let mut g = self.graph.write();
                    if let Err(e) = g.update_question_status(q_id, QuestionStatus::TimedOut) {
                        tracing::warn!("Failed to dismiss question {q_id}: {e}");
                    }
                    g.release_claim(q_id);
                }
            }
            Action::ScrollToBottom => {
                self.tui_state.scroll_mode = crate::tui::ScrollMode::Auto;
                self.tui_state.scroll_offset = u16::MAX;
            }
            Action::None => {}
        }
        Ok(())
    }

    /// Apply a scroll action, clamping immediately to prevent over-scroll
    /// accumulation. Re-enables autoscroll when the user scrolls to the bottom.
    fn handle_scroll(&mut self, action: &Action, page_size: u16) {
        self.tui_state.scroll_mode = crate::tui::ScrollMode::Manual;
        let going_up = matches!(action, Action::ScrollUp | Action::PageUp);
        if going_up {
            if let Some(ref mut d) = self.tui_state.agent_display {
                d.revealed_chars = usize::MAX;
            }
        }
        let delta = match action {
            Action::ScrollUp | Action::ScrollDown => 3,
            _ => page_size,
        };
        let new_offset = if going_up {
            self.tui_state.scroll_offset.saturating_sub(delta)
        } else {
            self.tui_state
                .scroll_offset
                .saturating_add(delta)
                .min(self.tui_state.max_scroll)
        };
        self.tui_state.scroll_offset = new_offset;
        // Re-enable autoscroll when the user scrolls to the bottom.
        if new_offset >= self.tui_state.max_scroll && self.tui_state.max_scroll > 0 {
            self.tui_state.scroll_mode = crate::tui::ScrollMode::Auto;
        }
    }

    /// Send a message: add user node to graph, spawn or wake agent loop.
    ///
    /// If a continuous agent is already running (idle or active), the `MessageAdded`
    /// event from `add_message()` wakes it. If no agent exists, spawn one.
    /// TUI feedback flows through the `EventBus`.
    fn handle_send_message(&mut self, text: String) -> anyhow::Result<()> {
        let trigger_text = text.clone();
        let user_msg_id = {
            let mut g = self.graph.write();
            let parent_id = g.active_leaf()?;
            let user_node = Node::Message {
                id: Uuid::new_v4(),
                role: Role::User,
                content: text,
                created_at: Utc::now(),
                model: None,
                input_tokens: None,
                output_tokens: None,
                stop_reason: None,
            };
            g.add_message(parent_id, user_node)?
        };

        self.dispatch_user_triggers(&trigger_text, user_msg_id);

        // UI-only state: scroll to follow the response.
        self.tui_state.scroll_mode = crate::tui::ScrollMode::Auto;
        self.tui_state.scroll_offset = u16::MAX;

        // Spawn agent only if none is running. Existing agents wake from
        // the `MessageAdded` event emitted by `add_message()` above.
        if self.agents.primary_agent_id.is_none() {
            let agent_id = Uuid::new_v4();
            let (tool_rx, cancel_token) = self.agents.register(agent_id);
            self.agents.primary_agent_id = Some(agent_id);

            // Emit an initial phase event for immediate TUI feedback.
            self.graph.read().emit(GraphEvent::AgentPhaseChanged {
                agent_id,
                phase: AgentPhase::CountingTokens,
            });

            let loop_config = agent::AgentLoopConfig {
                graph: Arc::clone(&self.graph),
                provider: Arc::clone(&self.provider),
                model: self.config.anthropic_model.clone(),
                max_tokens: self.config.max_tokens,
                max_context_tokens: self.config.max_context_tokens,
                max_tool_loop_iterations: self.config.max_tool_loop_iterations,
                tools: crate::tool_executor::registered_tool_definitions(),
                anchor_id: user_msg_id,
                agent_id,
            };

            agent::spawn_agent_loop(loop_config, self.task_tx.clone(), tool_rx, cancel_token);
        }

        Ok(())
    }

    /// Dispatch user triggers through the same pipeline as LLM tool calls.
    fn dispatch_user_triggers(&mut self, text: &str, user_msg_id: Uuid) {
        for trigger in crate::tools::parse_triggers(text) {
            let args = crate::tools::parse_user_trigger_args(&trigger.tool_name, &trigger.args);
            let tool_call_id = Uuid::new_v4();
            self.handle_tool_call_dispatched(tool_call_id, user_msg_id, args, None);
        }
    }

    /// Seed the status-bar token counters from the analytics DB.
    async fn seed_token_usage(&mut self) {
        let Some(ref store) = self.token_store else {
            return;
        };
        let totals = store.lifetime_totals().await.unwrap_or_default();
        self.tui_state.token_usage.input = AnimatedCounter {
            current: totals.input,
            target: totals.input,
        };
        self.tui_state.token_usage.output = AnimatedCounter {
            current: totals.output,
            target: totals.output,
        };
    }

    /// Drain all pending graph events before rendering. Events emitted within
    /// the current `tokio::select!` iteration (e.g., by `handle_task_message`)
    /// are processed here so TUI state is up-to-date before the frame is drawn.
    fn drain_pending_events(&mut self) {
        let Some(mut rx) = self.event_rx.take() else {
            return;
        };
        while let Ok(event) = rx.try_recv() {
            self.handle_graph_event(&event);
        }
        self.event_rx = Some(rx);
    }

    pub(super) fn save(&self) -> anyhow::Result<()> {
        let mut metadata = self.metadata.clone();
        metadata.last_modified = Utc::now();
        let g = self.graph.read();
        persistence::save_conversation(&metadata.id, &metadata, &g)
    }
}
