use crate::graph::tool_types::ToolCallStatus;
use crate::graph::{EdgeKind, Node};
use crate::llm::ChatConfig;
use crate::tasks::TaskMessage;
use crate::tui::input::{self, Action};
use crate::tui::ui;

use chrono::Utc;
use crossterm::event::{Event, EventStream};
use futures::StreamExt;
use ratatui::prelude::*;
use std::collections::HashSet;
use std::io;
use uuid::Uuid;

use super::App;

impl App {
    /// Run the agent loop: stream LLM response, dispatch tool calls, wait for
    /// results, and repeat until the LLM stops requesting tools or the iteration
    /// limit is reached.
    pub(super) async fn run_agent_loop(
        &mut self,
        config: &ChatConfig,
        terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
        event_stream: &mut EventStream,
    ) -> anyhow::Result<()> {
        for _ in 0..self.config.max_tool_loop_iterations {
            let (system_prompt, messages) = self.build_context().await?;
            let loop_config = ChatConfig {
                system_prompt,
                ..config.clone()
            };

            let result = self
                .stream_llm_response(messages, &loop_config, terminal, event_stream)
                .await?;

            if self.tui_state.should_quit {
                break;
            }

            if !result.response.is_empty() || !result.tool_use_records.is_empty() {
                let leaf = self
                    .graph
                    .branch_leaf(self.graph.active_branch())
                    .ok_or_else(|| anyhow::anyhow!("No leaf node for active branch"))?;
                let assistant_id = Uuid::new_v4();
                let assistant_node = Node::Message {
                    id: assistant_id,
                    role: crate::graph::Role::Assistant,
                    content: result.response,
                    created_at: Utc::now(),
                    model: Some(loop_config.model.clone()),
                    input_tokens: None,
                    output_tokens: result.output_tokens,
                };
                self.graph.add_message(leaf, assistant_node)?;

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

                let is_tool_use = result.stop_reason.as_deref() == Some("tool_use")
                    && !result.tool_use_records.is_empty();

                if !is_tool_use {
                    break;
                }

                let mut pending_ids = HashSet::new();
                for record in &result.tool_use_records {
                    let args =
                        crate::graph::parse_tool_arguments(&record.name, &record.input);
                    let api_id = Some(record.api_id.clone());
                    self.handle_tool_call_dispatched(
                        record.tool_call_id,
                        assistant_id,
                        args,
                        api_id,
                    );
                    pending_ids.insert(record.tool_call_id);
                }

                self.wait_for_tool_completions(&mut pending_ids, terminal, event_stream)
                    .await?;

                if self.tui_state.should_quit {
                    break;
                }
            } else {
                break;
            }
        }

        self.tui_state.streaming_response = None;
        self.tui_state.status_message = None;
        self.save()?;
        Ok(())
    }

    /// Wait for all pending tool calls to complete, keeping the TUI responsive.
    async fn wait_for_tool_completions(
        &mut self,
        pending_ids: &mut HashSet<Uuid>,
        terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
        event_stream: &mut EventStream,
    ) -> anyhow::Result<()> {
        self.tui_state.status_message =
            Some(format!("Executing {} tool call(s)...", pending_ids.len()));
        terminal.draw(|frame| ui::draw(frame, &self.graph, &mut self.tui_state))?;

        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(60);

        while !pending_ids.is_empty() {
            tokio::select! {
                biased;

                maybe_event = event_stream.next() => {
                    if let Some(Ok(Event::Key(key))) = maybe_event {
                        let action = input::handle_key_event(key, &mut self.tui_state);
                        if matches!(action, Action::Quit) {
                            self.tui_state.should_quit = true;
                            break;
                        }
                    }
                }

                Some(task_msg) = self.task_rx.recv() => {
                    if let TaskMessage::ToolCallCompleted { tool_call_id, content, is_error } = task_msg {
                        self.handle_tool_call_completed(tool_call_id, content, is_error);
                        pending_ids.remove(&tool_call_id);
                        self.tui_state.status_message = if pending_ids.is_empty() {
                            None
                        } else {
                            Some(format!("Executing {} tool call(s)...", pending_ids.len()))
                        };
                    } else {
                        self.handle_task_message(task_msg);
                    }
                }

                () = tokio::time::sleep_until(deadline) => {
                    for tc_id in pending_ids.drain() {
                        let _ = self.graph.update_tool_call_status(
                            tc_id,
                            ToolCallStatus::Failed,
                            Some(Utc::now()),
                        );
                        let result_id = Uuid::new_v4();
                        let result_node = Node::ToolResult {
                            id: result_id,
                            tool_call_id: tc_id,
                            content: "Tool execution timed out".to_string(),
                            is_error: true,
                            created_at: Utc::now(),
                        };
                        self.graph.add_node(result_node);
                        let _ = self.graph.add_edge(result_id, tc_id, EdgeKind::Produced);
                    }
                    self.tui_state.status_message = Some("Tool call(s) timed out".to_string());
                    break;
                }
            }
            terminal.draw(|frame| ui::draw(frame, &self.graph, &mut self.tui_state))?;
        }

        Ok(())
    }
}
