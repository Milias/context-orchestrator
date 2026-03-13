mod agent_loop;
mod context;
mod streaming;
mod task_handler;
mod think_splitter;

use crate::config::AppConfig;
use crate::graph::{ConversationGraph, Node, Role};
use crate::llm::{BackgroundLlmConfig, ChatConfig, ChatMessage, LlmProvider};
use crate::persistence::{self, ConversationMetadata};
use crate::tasks::{self, ContextSnapshot, TaskMessage};
use crate::tui::input::{self, Action};
use crate::tui::ui;
use crate::tui::{self, TuiState};

use chrono::Utc;
use crossterm::event::{Event, EventStream, KeyEventKind};
use futures::StreamExt;
use ratatui::prelude::*;
use std::io;
use std::sync::Arc;
use tokio::sync::{mpsc, Semaphore};
use uuid::Uuid;

pub struct App {
    config: AppConfig,
    graph: ConversationGraph,
    metadata: ConversationMetadata,
    provider: Arc<dyn LlmProvider>,
    background_semaphore: Arc<Semaphore>,
    tui_state: TuiState,
    task_rx: mpsc::UnboundedReceiver<TaskMessage>,
    task_tx: mpsc::UnboundedSender<TaskMessage>,
}

impl App {
    pub fn new(
        config: AppConfig,
        graph: ConversationGraph,
        metadata: ConversationMetadata,
        provider: Arc<dyn LlmProvider>,
    ) -> Self {
        let (task_tx, task_rx) = mpsc::unbounded_channel();
        let background_semaphore = Arc::new(Semaphore::new(config.background_max_concurrent));
        Self {
            config,
            graph,
            metadata,
            provider,
            background_semaphore,
            tui_state: TuiState::new(),
            task_rx,
            task_tx,
        }
    }

    fn snapshot_context(&self, trigger_message_id: Uuid) -> ContextSnapshot {
        let history = self
            .graph
            .get_branch_history(self.graph.active_branch())
            .unwrap_or_default();

        let messages: Vec<ChatMessage> = history
            .iter()
            .filter_map(|node| match node {
                Node::Message { role, content, .. } => {
                    let api_role = match role {
                        Role::User => "user",
                        Role::Assistant => "assistant",
                        Role::System => return None,
                    };
                    Some(ChatMessage::text(api_role, content))
                }
                _ => None,
            })
            .collect();

        let tools = self
            .graph
            .nodes_by(|n| matches!(n, Node::Tool { .. }))
            .into_iter()
            .filter_map(|n| {
                if let Node::Tool {
                    name, description, ..
                } = n
                {
                    Some(crate::tasks::ToolSnapshot {
                        name: name.clone(),
                        description: description.clone(),
                    })
                } else {
                    None
                }
            })
            .collect();

        ContextSnapshot {
            messages,
            tools,
            trigger_message_id,
        }
    }

    pub async fn run(mut self) -> anyhow::Result<()> {
        let mut terminal = tui::setup_terminal()?;
        let mut event_stream = EventStream::new();

        // Spawn background tasks
        tasks::spawn_git_watcher(self.task_tx.clone());
        tasks::spawn_tool_discovery(self.task_tx.clone());
        tasks::spawn_context_summarization(self.task_tx.clone());

        terminal.draw(|frame| ui::draw(frame, &self.graph, &mut self.tui_state))?;

        loop {
            if self.tui_state.should_quit {
                break;
            }

            tokio::select! {
                maybe_event = event_stream.next() => {
                    if let Some(Ok(Event::Key(key))) = maybe_event {
                        if key.kind != KeyEventKind::Press { continue; }
                        let action = input::handle_key_event(key, &mut self.tui_state);
                        match action {
                            Action::Quit => {
                                self.save()?;
                                break;
                            }
                            Action::SendMessage(text) => {
                                self.handle_send_message(text, &mut terminal, &mut event_stream).await?;
                            }
                            Action::ScrollUp | Action::ScrollDown => {
                                self.tui_state.scroll_offset = if matches!(action, Action::ScrollUp) {
                                    self.tui_state.scroll_offset.saturating_sub(3)
                                } else {
                                    self.tui_state.scroll_offset.saturating_add(3)
                                };
                            }
                            Action::PageUp | Action::PageDown => {
                                let page = terminal.size()?.height / 2;
                                self.tui_state.scroll_offset = if matches!(action, Action::PageUp) {
                                    self.tui_state.scroll_offset.saturating_sub(page)
                                } else {
                                    self.tui_state.scroll_offset.saturating_add(page)
                                };
                            }
                            Action::None => {}
                        }
                    }
                }
                Some(task_msg) = self.task_rx.recv() => {
                    self.handle_task_message(task_msg);
                }
            }

            terminal.draw(|frame| ui::draw(frame, &self.graph, &mut self.tui_state))?;
        }

        tui::restore_terminal(terminal)?;
        Ok(())
    }

    async fn handle_send_message(
        &mut self,
        text: String,
        terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
        event_stream: &mut EventStream,
    ) -> anyhow::Result<()> {
        self.tui_state.error_message = None;

        let parent_id = self
            .graph
            .branch_leaf(self.graph.active_branch())
            .ok_or_else(|| anyhow::anyhow!("No leaf node for active branch"))?;

        let single = vec![ChatMessage::text("user", &text)];
        let user_tokens = match self
            .provider
            .count_tokens(&single, &self.config.anthropic_model, None, &[])
            .await
        {
            Ok(count) => Some(count),
            Err(e) => {
                self.tui_state.error_message = Some(format!("Token count failed: {e}"));
                None
            }
        };

        let text_for_triggers = text.clone();
        let user_node = Node::Message {
            id: Uuid::new_v4(),
            role: Role::User,
            content: text,
            created_at: Utc::now(),
            model: None,
            input_tokens: user_tokens,
            output_tokens: None,
        };
        let user_msg_id = self.graph.add_message(parent_id, user_node)?;

        self.spawn_tool_triggers(&text_for_triggers, user_msg_id);

        let config = ChatConfig {
            system_prompt: None,
            tools: crate::tool_executor::registered_tool_definitions(),
            ..ChatConfig::from_app_config(&self.config)
        };

        self.run_agent_loop(&config, terminal, event_stream).await
    }

    fn spawn_tool_triggers(&self, text: &str, user_msg_id: Uuid) {
        for trigger in crate::tools::parse_triggers(text) {
            let snapshot = self.snapshot_context(user_msg_id);
            crate::tools::spawn_tool_extraction(
                trigger,
                snapshot,
                Arc::clone(&self.provider),
                Arc::clone(&self.background_semaphore),
                BackgroundLlmConfig::from_app_config(&self.config),
                self.task_tx.clone(),
            );
        }
    }

    fn save(&self) -> anyhow::Result<()> {
        let mut metadata = self.metadata.clone();
        metadata.last_modified = Utc::now();
        persistence::save_conversation(&metadata.id, &metadata, &self.graph)
    }
}
