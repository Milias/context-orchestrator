mod streaming;
mod task_handler;
mod think_splitter;

use crate::config::AppConfig;
use crate::graph::tool_types::ToolCallStatus;
use crate::graph::{ConversationGraph, EdgeKind, Node, Role};
use crate::llm::{
    BackgroundLlmConfig, ChatConfig, ChatContent, ChatMessage, ContentBlock, LlmProvider, RawJson,
};
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

    async fn build_context(&self) -> anyhow::Result<(Option<String>, Vec<ChatMessage>)> {
        let history = self.graph.get_branch_history(self.graph.active_branch())?;

        let mut system_prompt = None;
        let mut messages = Vec::new();

        for node in history {
            match node {
                Node::SystemDirective { content, .. } => {
                    system_prompt = Some(content.clone());
                }
                Node::Message {
                    id, role, content, ..
                } => match role {
                    Role::System => {}
                    Role::User => {
                        messages.push(ChatMessage::text("user", content));
                    }
                    Role::Assistant => {
                        let (asst_msg, result_msgs) =
                            self.build_assistant_message_with_tools(*id, content);
                        messages.push(asst_msg);
                        messages.extend(result_msgs);
                    }
                },
                // Non-conversation node types are skipped in LLM context
                Node::WorkItem { .. }
                | Node::GitFile { .. }
                | Node::Tool { .. }
                | Node::BackgroundTask { .. }
                | Node::ThinkBlock { .. }
                | Node::ToolCall { .. }
                | Node::ToolResult { .. } => {}
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
            let total_chars: usize = messages.iter().map(|m| m.content.char_len()).sum();
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
                current_chars -= removed.content.char_len();
            }
        }

        // Drop orphaned tool_result user messages at the front after truncation.
        // The Anthropic API rejects tool_result blocks without a preceding tool_use.
        while messages.len() > 1 && messages[0].role == "user" {
            let all_results = matches!(&messages[0].content,
                ChatContent::Blocks(b) if b.iter().all(|b| matches!(b, ContentBlock::ToolResult { .. })));
            if all_results {
                messages.remove(0);
            } else {
                break;
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
        let parent_id = self
            .graph
            .branch_leaf(self.graph.active_branch())
            .ok_or_else(|| anyhow::anyhow!("No leaf node for active branch"))?;

        let single = vec![ChatMessage::text("user", &text)];
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

        let result = self
            .stream_llm_response(messages, &config, terminal, event_stream)
            .await?;

        if !result.response.is_empty() || !result.tool_use_records.is_empty() {
            let leaf = self
                .graph
                .branch_leaf(self.graph.active_branch())
                .ok_or_else(|| anyhow::anyhow!("No leaf node for active branch"))?;
            let assistant_id = Uuid::new_v4();
            let assistant_node = Node::Message {
                id: assistant_id,
                role: Role::Assistant,
                content: result.response,
                created_at: Utc::now(),
                model: Some(config.model.clone()),
                input_tokens: None,
                output_tokens: result.output_tokens,
            };
            self.graph.add_message(leaf, assistant_node)?;

            for record in &result.tool_use_records {
                let args = crate::graph::parse_tool_arguments(&record.name, &record.input);
                let api_id = Some(record.api_id.clone());
                self.handle_tool_call_dispatched(record.tool_call_id, assistant_id, args, api_id);
            }

            if !result.think_text.is_empty() {
                let think_node = Node::ThinkBlock {
                    id: Uuid::new_v4(),
                    content: result.think_text,
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

    /// Build assistant `ChatMessage` with `ToolUse` blocks and any following
    /// user `ToolResult` messages. Ensures Anthropic API tool call/result pairing.
    fn build_assistant_message_with_tools(
        &self,
        message_id: Uuid,
        text_content: &str,
    ) -> (ChatMessage, Vec<ChatMessage>) {
        let tool_call_ids = self.graph.sources_by_edge(message_id, EdgeKind::Invoked);
        let mut tool_use_blocks = Vec::new();
        let mut result_blocks = Vec::new();
        for tc_id in &tool_call_ids {
            let Some(Node::ToolCall {
                status,
                arguments,
                api_tool_use_id,
                ..
            }) = self.graph.node(*tc_id)
            else {
                continue;
            };
            if *status != ToolCallStatus::Completed && *status != ToolCallStatus::Failed {
                continue;
            }

            // Take only the first ToolResult per ToolCall (Anthropic API expects 1:1 pairing).
            let result_id = self
                .graph
                .sources_by_edge(*tc_id, EdgeKind::Produced)
                .into_iter()
                .next();
            let Some(result_id) = result_id else {
                continue;
            };
            let Some(Node::ToolResult {
                content, is_error, ..
            }) = self.graph.node(result_id)
            else {
                continue;
            };
            let use_id = api_tool_use_id.clone().unwrap_or_else(|| tc_id.to_string());
            tool_use_blocks.push(ContentBlock::ToolUse {
                id: use_id.clone(),
                name: arguments.tool_name().to_string(),
                input: RawJson(arguments.to_input_json()),
            });
            result_blocks.push(ContentBlock::ToolResult {
                tool_use_id: use_id,
                content: content.clone(),
                is_error: *is_error,
            });
        }

        if tool_use_blocks.is_empty() {
            return (ChatMessage::text("assistant", text_content), vec![]);
        }
        let mut blocks = vec![ContentBlock::Text {
            text: text_content.to_string(),
        }];
        blocks.extend(tool_use_blocks);
        let asst = ChatMessage {
            role: "assistant".to_string(),
            content: ChatContent::Blocks(blocks),
        };
        let results = ChatMessage {
            role: "user".to_string(),
            content: ChatContent::Blocks(result_blocks),
        };
        (asst, vec![results])
    }

    fn save(&self) -> anyhow::Result<()> {
        let mut metadata = self.metadata.clone();
        metadata.last_modified = Utc::now();
        persistence::save_conversation(&metadata.id, &metadata, &self.graph)
    }
}
