mod agent_loop;
mod agent_streaming;
mod context;
mod task_handler;
mod think_splitter;

use crate::config::AppConfig;
use crate::graph::{ConversationGraph, Node, Role};
use crate::llm::LlmProvider;
use crate::persistence::{self, ConversationMetadata};
use crate::tasks::{self, AgentToolResult, TaskMessage};
use crate::tui::input::{self, Action};
use crate::tui::ui;
use crate::tui::{self, AgentDisplayState, TuiState};

use chrono::Utc;
use crossterm::event::{Event, EventStream, KeyEventKind};
use futures::StreamExt;
use parking_lot::RwLock;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
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
    /// Sender for forwarding tool completions to the running agent.
    agent_tool_tx: Option<mpsc::UnboundedSender<AgentToolResult>>,
    /// Root cancellation token for the running agent. Cancelling this propagates
    /// to all child tokens (tool executions, streaming retries).
    cancel_token: Option<CancellationToken>,
    /// Per-task cancellation tokens, keyed by `tool_call_id`. Child tokens of
    /// `cancel_token` — cancelling the parent propagates to all children.
    task_tokens: HashMap<Uuid, CancellationToken>,
    /// Node IDs of currently running agent phases (`BackgroundTask` nodes).
    /// Multiple phases can be active simultaneously (e.g. token counting + context building).
    active_phase_ids: HashSet<Uuid>,
}

impl App {
    pub fn new(
        config: AppConfig,
        graph: ConversationGraph,
        metadata: ConversationMetadata,
        provider: Arc<dyn LlmProvider>,
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
            agent_tool_tx: None,
            cancel_token: None,
            task_tokens: HashMap::new(),
            active_phase_ids: HashSet::new(),
        }
    }

    pub async fn run(mut self) -> anyhow::Result<()> {
        let mut terminal = tui::setup_terminal()?;
        let mut event_stream = EventStream::new();
        let mut spinner_interval = tokio::time::interval(Duration::from_millis(80));
        spinner_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut sigterm =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;

        // Stale Running tasks from a previous crash → mark as Failed
        self.graph.write().expire_stale_tasks();

        // Spawn background tasks
        tasks::spawn_git_watcher(self.task_tx.clone());
        tasks::spawn_tool_discovery(self.task_tx.clone());
        tasks::spawn_context_summarization(self.task_tx.clone());

        {
            let g = self.graph.read();
            terminal.draw(|frame| ui::draw(frame, &g, &mut self.tui_state))?;
        }

        loop {
            if self.tui_state.should_quit {
                break;
            }

            let agent_active = self.tui_state.agent_display.is_some();

            tokio::select! {
                maybe_event = event_stream.next() => {
                    if let Some(Ok(Event::Key(key))) = maybe_event {
                        if key.kind != KeyEventKind::Press { continue; }
                        let action = {
                            let g = self.graph.read();
                            input::handle_key_event(key, &mut self.tui_state, &g)
                        };
                        match action {
                            Action::Quit => {
                                if let Some(ref token) = self.cancel_token {
                                    token.cancel();
                                }
                                self.agent_tool_tx = None;
                                self.graph.write().stop_running_tasks();
                                self.save()?;
                                break;
                            }
                            Action::SendMessage(text) => {
                                if agent_active {
                                    self.tui_state.input_text = text;
                                    self.tui_state.input_cursor =
                                        self.tui_state.input_text.chars().count();
                                } else {
                                    self.handle_send_message(text)?;
                                }
                            }
                            Action::ScrollUp | Action::ScrollDown => {
                                self.tui_state.scroll_mode = crate::tui::ScrollMode::Manual;
                                self.tui_state.scroll_offset = if matches!(action, Action::ScrollUp) {
                                    self.tui_state.scroll_offset.saturating_sub(3)
                                } else {
                                    self.tui_state.scroll_offset.saturating_add(3)
                                };
                            }
                            Action::PageUp | Action::PageDown => {
                                self.tui_state.scroll_mode = crate::tui::ScrollMode::Manual;
                                let page = terminal.size()?.height / 2;
                                self.tui_state.scroll_offset = if matches!(action, Action::PageUp) {
                                    self.tui_state.scroll_offset.saturating_sub(page)
                                } else {
                                    self.tui_state.scroll_offset.saturating_add(page)
                                };
                            }
                            Action::CancelTask(id) => {
                                self.cancel_task(id);
                            }
                            Action::None => {}
                        }
                    }
                }
                Some(task_msg) = self.task_rx.recv() => {
                    self.handle_task_message(task_msg);
                }
                _ = spinner_interval.tick(), if agent_active => {
                    if let Some(ref mut display) = self.tui_state.agent_display {
                        display.spinner_tick = display.spinner_tick.wrapping_add(1);
                    }
                }
                _ = sigterm.recv() => {
                    if let Some(ref token) = self.cancel_token {
                        token.cancel();
                    }
                    self.agent_tool_tx = None;
                    self.graph.write().stop_running_tasks();
                    let _ = self.save();
                    break;
                }
            }

            {
                let g = self.graph.read();
                terminal.draw(|frame| ui::draw(frame, &g, &mut self.tui_state))?;
            }
        }

        tui::restore_terminal(terminal)?;
        Ok(())
    }

    /// Send a message: add user node to graph, spawn agent loop, return immediately.
    fn handle_send_message(&mut self, text: String) -> anyhow::Result<()> {
        self.tui_state.error_message = None;

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

        // Set UI state for immediate feedback
        self.tui_state.agent_display = Some(AgentDisplayState::default());
        self.tui_state.status_message = Some("Counting tokens...".to_string());
        self.tui_state.scroll_mode = crate::tui::ScrollMode::Auto;
        self.tui_state.scroll_offset = u16::MAX;

        // Create channels and cancellation for agent ↔ main loop communication
        let (agent_tool_tx, agent_tool_rx) = mpsc::unbounded_channel();
        let cancel_token = CancellationToken::new();
        self.agent_tool_tx = Some(agent_tool_tx);
        self.cancel_token = Some(cancel_token.clone());
        self.task_tokens.clear();

        let loop_config = agent_loop::AgentLoopConfig {
            model: self.config.anthropic_model.clone(),
            max_tokens: self.config.max_tokens,
            max_context_tokens: self.config.max_context_tokens,
            max_tool_loop_iterations: self.config.max_tool_loop_iterations,
            tools: crate::tool_executor::registered_tool_definitions(),
        };

        agent_loop::spawn_agent_loop(
            Arc::clone(&self.graph),
            Arc::clone(&self.provider),
            loop_config,
            user_msg_id,
            self.task_tx.clone(),
            agent_tool_rx,
            cancel_token,
        );

        Ok(())
    }

    /// Dispatch user triggers through the same pipeline as LLM tool calls.
    /// All triggers go through `handle_tool_call_dispatched` → `spawn_tool_execution` → `ToolCallCompleted`.
    fn dispatch_user_triggers(&mut self, text: &str, user_msg_id: Uuid) {
        for trigger in crate::tools::parse_triggers(text) {
            let args = crate::tools::parse_user_trigger_args(&trigger.tool_name, &trigger.args);
            let tool_call_id = Uuid::new_v4();
            self.handle_tool_call_dispatched(tool_call_id, user_msg_id, args, None);
        }
    }

    pub(super) fn save(&self) -> anyhow::Result<()> {
        let mut metadata = self.metadata.clone();
        metadata.last_modified = Utc::now();
        let g = self.graph.read();
        persistence::save_conversation(&metadata.id, &metadata, &g)
    }
}
