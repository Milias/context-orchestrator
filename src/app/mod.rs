mod task_handler;

use crate::config::AppConfig;
use crate::graph::{ConversationGraph, EdgeKind, Node, Role};
use crate::llm::{BackgroundLlmConfig, ChatConfig, ChatMessage, LlmProvider, StreamChunk};
use crate::persistence::{self, ConversationMetadata};
use crate::tasks::{self, ContextSnapshot, TaskMessage};
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
                | Node::BackgroundTask { .. }
                | Node::ThinkBlock { .. } => {}
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

        let (response, think_text, output_tokens) = self
            .stream_llm_response(messages, &config, terminal, event_stream)
            .await?;

        if !response.is_empty() {
            let leaf = self.graph.branch_leaf(self.graph.active_branch()).unwrap();
            let assistant_id = Uuid::new_v4();
            let assistant_node = Node::Message {
                id: assistant_id,
                role: Role::Assistant,
                content: response,
                created_at: Utc::now(),
                model: Some(config.model.clone()),
                input_tokens: None,
                output_tokens,
            };
            self.graph.add_message(leaf, assistant_node)?;

            if !think_text.is_empty() {
                let think_node = Node::ThinkBlock {
                    id: Uuid::new_v4(),
                    content: think_text,
                    parent_message_id: assistant_id,
                    created_at: Utc::now(),
                };
                let think_id = self.graph.add_node(think_node);
                self.graph
                    .add_edge(think_id, assistant_id, EdgeKind::ThinkingOf)?;
            }
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
    ) -> anyhow::Result<(String, String, Option<u32>)> {
        self.tui_state.streaming_response = Some(String::new());
        self.tui_state.status_message = Some("Waiting for response...".to_string());
        self.tui_state.scroll_offset = u16::MAX;
        terminal.draw(|frame| ui::draw(frame, &self.graph, &mut self.tui_state))?;

        let mut stream = match self.provider.chat(messages, config).await {
            Ok(s) => s,
            Err(e) => {
                self.tui_state.streaming_response = None;
                self.tui_state.status_message = Some(format!("Error: {e}"));
                return Ok((String::new(), String::new(), None));
            }
        };

        let mut think_splitter = ThinkSplitter::new();
        let mut output_tokens = None;

        loop {
            tokio::select! {
                maybe_chunk = stream.next() => {
                    match maybe_chunk {
                        Some(Ok(StreamChunk::TextDelta(text))) => {
                            think_splitter.push(&text);
                            self.tui_state.streaming_response = Some(think_splitter.visible().to_string());
                            self.tui_state.status_message = Some(
                                if think_splitter.is_thinking() { "Thinking..." } else { "Receiving..." }.to_string()
                            );
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

        let (clean_response, think_content) = think_splitter.finish();
        Ok((clean_response, think_content, output_tokens))
    }

    fn save(&self) -> anyhow::Result<()> {
        let mut metadata = self.metadata.clone();
        metadata.last_modified = Utc::now();
        persistence::save_conversation(&metadata.id, &metadata, &self.graph)
    }
}

/// Incrementally splits streaming text into visible content and think blocks.
/// Handles multiple `<think>...</think>` blocks and single-chunk edge cases.
struct ThinkSplitter {
    visible: String,
    think_blocks: Vec<String>,
    buffer: String,
    in_think: bool,
}

impl ThinkSplitter {
    fn new() -> Self {
        Self {
            visible: String::new(),
            think_blocks: Vec::new(),
            buffer: String::new(),
            in_think: false,
        }
    }

    fn push(&mut self, chunk: &str) {
        self.buffer.push_str(chunk);
        self.drain_buffer();
    }

    fn drain_buffer(&mut self) {
        loop {
            if self.in_think {
                match self.buffer.find("</think>") {
                    Some(end) => {
                        self.think_blocks.push(self.buffer[..end].to_string());
                        self.buffer = self.buffer[end + 8..].to_string();
                        self.in_think = false;
                    }
                    None => break,
                }
            } else if let Some(start) = self.buffer.find("<think>") {
                self.visible.push_str(&self.buffer[..start]);
                self.buffer = self.buffer[start + 7..].to_string();
                self.in_think = true;
            } else {
                // Keep a tail that could be a partial `<think>` tag
                let safe = self.buffer.len().saturating_sub(6);
                self.visible.push_str(&self.buffer[..safe]);
                self.buffer = self.buffer[safe..].to_string();
                break;
            }
        }
    }

    fn visible(&self) -> &str {
        &self.visible
    }

    fn is_thinking(&self) -> bool {
        self.in_think
    }

    /// Finalize: flush remaining buffer and return (visible, `think_content`).
    fn finish(mut self) -> (String, String) {
        if self.in_think {
            // Unclosed think block — treat remaining buffer as think content
            self.think_blocks.push(std::mem::take(&mut self.buffer));
        } else {
            self.visible.push_str(&self.buffer);
        }
        let think = self.think_blocks.join("\n");
        (self.visible, think)
    }
}

#[cfg(test)]
mod tests {
    use super::ThinkSplitter;

    #[test]
    fn no_think_tags() {
        let mut s = ThinkSplitter::new();
        s.push("Hello world");
        let (visible, think) = s.finish();
        assert_eq!(visible, "Hello world");
        assert!(think.is_empty());
    }

    #[test]
    fn single_think_block() {
        let mut s = ThinkSplitter::new();
        s.push("<think>reasoning</think>answer");
        let (visible, think) = s.finish();
        assert_eq!(visible, "answer");
        assert_eq!(think, "reasoning");
    }

    #[test]
    fn think_block_across_chunks() {
        let mut s = ThinkSplitter::new();
        s.push("<thi");
        s.push("nk>reas");
        s.push("oning</thi");
        s.push("nk>answer");
        let (visible, think) = s.finish();
        assert_eq!(visible, "answer");
        assert_eq!(think, "reasoning");
    }

    #[test]
    fn multiple_think_blocks() {
        let mut s = ThinkSplitter::new();
        s.push("before<think>first</think>middle<think>second</think>after");
        let (visible, think) = s.finish();
        assert_eq!(visible, "beforemiddleafter");
        assert_eq!(think, "first\nsecond");
    }

    #[test]
    fn unclosed_think_block() {
        let mut s = ThinkSplitter::new();
        s.push("visible<think>partial thinking");
        let (visible, think) = s.finish();
        assert_eq!(visible, "visible");
        assert_eq!(think, "partial thinking");
    }

    #[test]
    fn is_thinking_state() {
        let mut s = ThinkSplitter::new();
        assert!(!s.is_thinking());
        s.push("<think>thinking");
        assert!(s.is_thinking());
        s.push("</think>done");
        assert!(!s.is_thinking());
    }
}
