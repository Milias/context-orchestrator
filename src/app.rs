use crate::config::AppConfig;
use crate::graph::{ConversationGraph, Node, Role};
use crate::llm::{ChatConfig, ChatMessage, LlmProvider, StreamChunk};
use crate::persistence::{self, ConversationMetadata};
use crate::tui::input::{self, Action};
use crate::tui::ui;
use crate::tui::{self, Focus, TuiState};

use chrono::Utc;
use crossterm::event::{Event, EventStream};
use futures::StreamExt;
use ratatui::prelude::*;
use std::io;
use uuid::Uuid;

pub struct App {
    config: AppConfig,
    graph: ConversationGraph,
    metadata: ConversationMetadata,
    provider: Box<dyn LlmProvider>,
    tui_state: TuiState,
}

impl App {
    pub fn new(
        config: AppConfig,
        graph: ConversationGraph,
        metadata: ConversationMetadata,
        provider: Box<dyn LlmProvider>,
    ) -> Self {
        let mut tui_state = TuiState::new();
        let branches = graph.branch_names();
        let active = graph.active_branch().to_string();
        if let Some(idx) = branches.iter().position(|b| b.as_str() == active) {
            tui_state.branch_list_selected = idx;
        }
        Self {
            config,
            graph,
            metadata,
            provider,
            tui_state,
        }
    }

    fn build_context(&self) -> anyhow::Result<(Option<String>, Vec<ChatMessage>)> {
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
            }
        }

        // Truncation: ~4 chars per token, limit to ~150K tokens
        let max_chars: usize = 600_000;
        let mut total_chars: usize = messages.iter().map(|m| m.content.len()).sum();
        while total_chars > max_chars && messages.len() > 1 {
            let removed = messages.remove(0);
            total_chars -= removed.content.len();
        }

        Ok((system_prompt, messages))
    }

    pub async fn run(mut self) -> anyhow::Result<()> {
        let mut terminal = tui::setup_terminal()?;
        let mut event_stream = EventStream::new();

        terminal.draw(|frame| ui::draw(frame, &self.graph, &self.tui_state))?;

        loop {
            if self.tui_state.should_quit {
                break;
            }

            tokio::select! {
                maybe_event = event_stream.next() => {
                    match maybe_event {
                        Some(Ok(Event::Key(key))) => {
                            let action = input::handle_key_event(key, &mut self.tui_state);
                            match action {
                                Action::Quit => {
                                    self.save()?;
                                    break;
                                }
                                Action::SendMessage(text) => {
                                    self.handle_send_message(text, &mut terminal, &mut event_stream).await?;
                                }
                                Action::CreateBranch(name) => {
                                    self.handle_create_branch(&name)?;
                                }
                                Action::SwitchBranch(index) => {
                                    self.handle_switch_branch(index)?;
                                }
                                Action::ToggleFocus => {
                                    self.tui_state.focus = match self.tui_state.focus {
                                        Focus::Input => Focus::BranchList,
                                        Focus::BranchList => Focus::Input,
                                    };
                                }
                                Action::ScrollUp => {
                                    self.tui_state.scroll_offset = self.tui_state.scroll_offset.saturating_sub(1);
                                }
                                Action::ScrollDown => {
                                    self.tui_state.scroll_offset += 1;
                                }
                                Action::None => {}
                            }
                        }
                        Some(Ok(Event::Resize(_, _))) => {}
                        _ => {}
                    }
                }
            }

            terminal.draw(|frame| ui::draw(frame, &self.graph, &self.tui_state))?;
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

        let user_node = Node::Message {
            id: Uuid::new_v4(),
            role: Role::User,
            content: text,
            created_at: Utc::now(),
            model: None,
            token_count: None,
        };
        self.graph.add_message(parent_id, user_node)?;

        let (system_prompt, messages) = self.build_context()?;
        let config = ChatConfig {
            system_prompt,
            ..ChatConfig::from_app_config(&self.config)
        };

        self.tui_state.streaming_response = Some(String::new());
        self.tui_state.status_message = Some("Waiting for response...".to_string());
        terminal.draw(|frame| ui::draw(frame, &self.graph, &self.tui_state))?;

        let mut stream = match self.provider.chat(messages, &config).await {
            Ok(s) => s,
            Err(e) => {
                self.tui_state.streaming_response = None;
                self.tui_state.status_message = Some(format!("Error: {}", e));
                return Ok(());
            }
        };

        let mut full_response = String::new();
        let mut output_tokens = None;
        let mut input_tokens = None;

        loop {
            tokio::select! {
                maybe_chunk = stream.next() => {
                    match maybe_chunk {
                        Some(Ok(StreamChunk::TextDelta(text))) => {
                            full_response.push_str(&text);
                            self.tui_state.streaming_response = Some(full_response.clone());
                            self.tui_state.status_message = Some("Receiving...".to_string());
                        }
                        Some(Ok(StreamChunk::Done { input_tokens: it, output_tokens: ot })) => {
                            input_tokens = it;
                            output_tokens = ot;
                            break;
                        }
                        Some(Ok(StreamChunk::Error(e))) => {
                            self.tui_state.status_message = Some(format!("API Error: {}", e));
                            break;
                        }
                        Some(Err(e)) => {
                            self.tui_state.status_message = Some(format!("Stream error: {}", e));
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

            terminal.draw(|frame| ui::draw(frame, &self.graph, &self.tui_state))?;
        }

        if !full_response.is_empty() {
            let leaf = self.graph.branch_leaf(self.graph.active_branch()).unwrap();
            let assistant_node = Node::Message {
                id: Uuid::new_v4(),
                role: Role::Assistant,
                content: full_response,
                created_at: Utc::now(),
                model: Some(config.model.clone()),
                token_count: output_tokens,
            };
            self.graph.add_message(leaf, assistant_node)?;
        }

        self.tui_state.streaming_response = None;
        self.tui_state.status_message = None;
        let _ = input_tokens; // tracked but not displayed in MVP

        self.save()?;
        Ok(())
    }

    fn handle_create_branch(&mut self, name: &str) -> anyhow::Result<()> {
        let fork_point = self
            .graph
            .branch_leaf(self.graph.active_branch())
            .ok_or_else(|| anyhow::anyhow!("No active branch leaf"))?;

        match self.graph.create_branch(name, fork_point) {
            Ok(()) => {
                self.graph.switch_branch(name)?;
                self.tui_state.status_message = Some(format!("Created branch: {}", name));
                self.save()?;
            }
            Err(e) => {
                self.tui_state.status_message = Some(format!("Error: {}", e));
            }
        }
        Ok(())
    }

    fn handle_switch_branch(&mut self, index: usize) -> anyhow::Result<()> {
        let branches = self.graph.branch_names();
        let clamped = index.min(branches.len().saturating_sub(1));
        self.tui_state.branch_list_selected = clamped;

        if let Some(name) = branches.get(clamped) {
            let name = name.to_string();
            self.graph.switch_branch(&name)?;
            self.tui_state.scroll_offset = 0;
        }
        Ok(())
    }

    fn save(&self) -> anyhow::Result<()> {
        let mut metadata = self.metadata.clone();
        metadata.last_modified = Utc::now();
        persistence::save_conversation(&metadata.id, &metadata, &self.graph)
    }
}
