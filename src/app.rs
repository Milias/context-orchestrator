use crate::config::AppConfig;
use crate::graph::{ConversationGraph, EdgeKind, Node, Role};
use crate::llm::{BackgroundLlmConfig, ChatConfig, ChatMessage, LlmProvider, StreamChunk};
use crate::persistence::{self, ConversationMetadata};
use crate::tasks::{self, ContextSnapshot, TaskMessage, ToolExtractionOutcome};
use crate::tui::input::{self, Action};
use crate::tui::ui;
use crate::tui::{self, TuiState};

use chrono::Utc;
use crossterm::event::{Event, EventStream};
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
                    Some(ChatMessage {
                        role: api_role.to_string(),
                        content: content.clone(),
                    })
                }
                _ => None,
            })
            .collect();

        let tools = self
            .graph
            .nodes_by(|n| matches!(n, Node::Tool { .. }))
            .into_iter()
            .filter_map(|n| match n {
                Node::Tool {
                    name, description, ..
                } => Some(crate::tasks::ToolSnapshot {
                    name: name.clone(),
                    description: description.clone(),
                }),
                _ => None,
            })
            .collect();

        ContextSnapshot {
            messages,
            tools,
            trigger_message_id,
        }
    }

    async fn build_context(&self) -> anyhow::Result<(Option<String>, Vec<ChatMessage>)> {
        let history = self.graph.get_branch_history(self.graph.active_branch())?;

        let mut system_prompt = None;
        let mut messages = Vec::new();

        for node in history {
            match node {
                Node::SystemDirective { content, .. } => {
                    system_prompt = Some(content.clone());
                }
                Node::Message { role, content, .. } => {
                    let api_role = match role {
                        Role::User => "user",
                        Role::Assistant => "assistant",
                        Role::System => continue,
                    };
                    messages.push(ChatMessage {
                        role: api_role.to_string(),
                        content: content.clone(),
                    });
                }
                // Non-conversation node types are skipped in LLM context
                Node::WorkItem { .. }
                | Node::GitFile { .. }
                | Node::Tool { .. }
                | Node::BackgroundTask { .. } => {}
            }
        }

        let max_tokens = self.config.max_context_tokens;
        let token_count = self
            .provider
            .count_tokens(
                &messages,
                &self.config.anthropic_model,
                system_prompt.as_deref(),
            )
            .await?;

        if token_count > max_tokens {
            let total_chars: usize = messages.iter().map(|m| m.content.len()).sum();
            let ratio = f64::from(max_tokens) / f64::from(token_count);
            // Truncation/sign-loss/precision-loss are acceptable here: total_chars and ratio
            // are both non-negative and the result fits comfortably in usize for any realistic
            // conversation size.
            #[allow(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                clippy::cast_precision_loss
            )]
            let target_chars = (total_chars as f64 * ratio) as usize;

            let mut current_chars = total_chars;
            while current_chars > target_chars && messages.len() > 1 {
                let removed = messages.remove(0);
                current_chars -= removed.content.len();
            }
        }

        Ok((system_prompt, messages))
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
                        let action = input::handle_key_event(key, &mut self.tui_state);
                        match action {
                            Action::Quit => {
                                self.save()?;
                                break;
                            }
                            Action::SendMessage(text) => {
                                self.handle_send_message(text, &mut terminal, &mut event_stream).await?;
                            }
                            Action::ScrollUp => {
                                self.tui_state.scroll_offset = self.tui_state.scroll_offset.saturating_sub(3);
                            }
                            Action::ScrollDown => {
                                self.tui_state.scroll_offset = self.tui_state.scroll_offset.saturating_add(3);
                            }
                            Action::PageUp => {
                                let page = terminal.size()?.height / 2;
                                self.tui_state.scroll_offset = self.tui_state.scroll_offset.saturating_sub(page);
                            }
                            Action::PageDown => {
                                let page = terminal.size()?.height / 2;
                                self.tui_state.scroll_offset = self.tui_state.scroll_offset.saturating_add(page);
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
        let parent_id = self
            .graph
            .branch_leaf(self.graph.active_branch())
            .ok_or_else(|| anyhow::anyhow!("No leaf node for active branch"))?;

        let single = vec![ChatMessage {
            role: "user".into(),
            content: text.clone(),
        }];
        let user_tokens = self
            .provider
            .count_tokens(&single, &self.config.anthropic_model, None)
            .await
            .ok();

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

        let (system_prompt, messages) = self.build_context().await?;
        let config = ChatConfig {
            system_prompt,
            ..ChatConfig::from_app_config(&self.config)
        };

        let (response, output_tokens) = self
            .stream_llm_response(messages, &config, terminal, event_stream)
            .await?;

        if !response.is_empty() {
            let leaf = self.graph.branch_leaf(self.graph.active_branch()).unwrap();
            let assistant_node = Node::Message {
                id: Uuid::new_v4(),
                role: Role::Assistant,
                content: response,
                created_at: Utc::now(),
                model: Some(config.model.clone()),
                input_tokens: None,
                output_tokens,
            };
            self.graph.add_message(leaf, assistant_node)?;
        }

        self.tui_state.streaming_response = None;
        self.tui_state.status_message = None;
        self.save()?;
        Ok(())
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

    async fn stream_llm_response(
        &mut self,
        messages: Vec<ChatMessage>,
        config: &ChatConfig,
        terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
        event_stream: &mut EventStream,
    ) -> anyhow::Result<(String, Option<u32>)> {
        self.tui_state.streaming_response = Some(String::new());
        self.tui_state.status_message = Some("Waiting for response...".to_string());
        self.tui_state.scroll_offset = u16::MAX;
        terminal.draw(|frame| ui::draw(frame, &self.graph, &mut self.tui_state))?;

        let mut stream = match self.provider.chat(messages, config).await {
            Ok(s) => s,
            Err(e) => {
                self.tui_state.streaming_response = None;
                self.tui_state.status_message = Some(format!("Error: {e}"));
                return Ok((String::new(), None));
            }
        };

        let mut full_response = String::new();
        let mut output_tokens = None;

        loop {
            tokio::select! {
                maybe_chunk = stream.next() => {
                    match maybe_chunk {
                        Some(Ok(StreamChunk::TextDelta(text))) => {
                            full_response.push_str(&text);
                            self.tui_state.streaming_response = Some(full_response.clone());
                            self.tui_state.status_message = Some("Receiving...".to_string());
                            self.tui_state.scroll_offset = u16::MAX;
                        }
                        Some(Ok(StreamChunk::Done { output_tokens: ot })) => {
                            output_tokens = ot;
                            break;
                        }
                        Some(Ok(StreamChunk::Error(e))) => {
                            self.tui_state.status_message = Some(format!("API Error: {e}"));
                            break;
                        }
                        Some(Err(e)) => {
                            self.tui_state.status_message = Some(format!("Stream error: {e}"));
                            break;
                        }
                        None => break,
                    }
                }
                maybe_event = event_stream.next() => {
                    if let Some(Ok(Event::Key(key))) = maybe_event {
                        if key.modifiers.contains(crossterm::event::KeyModifiers::CONTROL)
                            && key.code == crossterm::event::KeyCode::Char('q')
                        {
                            self.tui_state.should_quit = true;
                            break;
                        }
                    }
                }
            }
            terminal.draw(|frame| ui::draw(frame, &self.graph, &mut self.tui_state))?;
        }

        Ok((full_response, output_tokens))
    }

    fn handle_task_message(&mut self, msg: TaskMessage) {
        match msg {
            TaskMessage::GitFilesUpdated(files) => {
                self.graph
                    .remove_nodes_by(|n| matches!(n, Node::GitFile { .. }));
                let root_id = self.graph.branch_leaf(self.graph.active_branch());
                for file in files {
                    let node = Node::GitFile {
                        id: Uuid::new_v4(),
                        path: file.path,
                        status: file.status,
                        updated_at: Utc::now(),
                    };
                    let node_id = self.graph.add_node(node);
                    if let Some(root) = root_id {
                        let _ = self.graph.add_edge(node_id, root, EdgeKind::Indexes);
                    }
                }
            }
            TaskMessage::ToolsDiscovered(tools) => {
                self.graph
                    .remove_nodes_by(|n| matches!(n, Node::Tool { .. }));
                let root_id = self.graph.branch_leaf(self.graph.active_branch());
                for tool in tools {
                    let node = Node::Tool {
                        id: Uuid::new_v4(),
                        name: tool.name,
                        description: tool.description,
                        updated_at: Utc::now(),
                    };
                    let node_id = self.graph.add_node(node);
                    if let Some(root) = root_id {
                        let _ = self.graph.add_edge(node_id, root, EdgeKind::Provides);
                    }
                }
            }
            TaskMessage::TaskStatusChanged {
                task_id,
                kind,
                status,
                description,
            } => {
                self.graph.upsert_node(Node::BackgroundTask {
                    id: task_id,
                    kind,
                    status,
                    description,
                    created_at: Utc::now(),
                    updated_at: Utc::now(),
                });
            }
            TaskMessage::ToolExtractionComplete {
                trigger_message_id,
                result,
            } => match result {
                ToolExtractionOutcome::Plan(plan) => {
                    let node = crate::tools::plan_result_to_node(&plan);
                    let node_id = self.graph.add_node(node);
                    let _ = self.graph.add_edge(
                        node_id,
                        trigger_message_id,
                        crate::tools::tool_result_edge_kind(),
                    );
                    self.tui_state.status_message =
                        Some(format!("Work item created: {}", plan.title));
                }
            },
        }
    }

    fn save(&self) -> anyhow::Result<()> {
        let mut metadata = self.metadata.clone();
        metadata.last_modified = Utc::now();
        persistence::save_conversation(&metadata.id, &metadata, &self.graph)
    }
}
