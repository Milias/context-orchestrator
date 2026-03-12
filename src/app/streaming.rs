use crate::llm::{ChatConfig, ChatMessage, StreamChunk};
use crate::tui::input::{self, Action};
use crate::tui::ui;

use super::think_splitter::ThinkSplitter;
use super::App;

use crossterm::event::{Event, EventStream};
use futures::StreamExt;
use ratatui::prelude::*;
use std::io;
use uuid::Uuid;

/// A `tool_use` block received during streaming, recorded for provenance after
/// the assistant node is created.
pub(super) struct ToolUseRecord {
    pub tool_call_id: Uuid,
    /// The Anthropic-assigned `tool_use` ID (e.g. `toolu_xxx`). Used in content
    /// blocks sent back to the API so `tool_use`/`tool_result` pairing is correct.
    pub api_id: String,
    pub name: String,
    pub input: String,
}

/// Return value from `stream_llm_response`.
#[derive(Default)]
pub(super) struct StreamResult {
    pub response: String,
    pub think_text: String,
    pub output_tokens: Option<u32>,
    pub tool_use_records: Vec<ToolUseRecord>,
    pub stop_reason: Option<String>,
}

impl App {
    pub(super) async fn stream_llm_response(
        &mut self,
        messages: Vec<ChatMessage>,
        config: &ChatConfig,
        terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
        event_stream: &mut EventStream,
    ) -> anyhow::Result<StreamResult> {
        self.tui_state.streaming_response = Some(String::new());
        self.tui_state.status_message = Some("Waiting for response...".to_string());
        self.tui_state.auto_scroll = true;
        self.tui_state.scroll_offset = u16::MAX;
        terminal.draw(|frame| ui::draw(frame, &self.graph, &mut self.tui_state))?;

        let mut stream = match self.provider.chat(messages, config).await {
            Ok(s) => s,
            Err(e) => {
                self.tui_state.streaming_response = None;
                self.tui_state.status_message = Some(format!("Error: {e}"));
                return Ok(StreamResult::default());
            }
        };

        let mut think_splitter = ThinkSplitter::new();
        let mut output_tokens = None;
        let mut tool_use_records = Vec::new();
        let mut stop_reason = None;
        loop {
            tokio::select! {
                biased; // event branch first so user input isn't starved by SSE bursts

                maybe_event = event_stream.next() => {
                    if let Some(Ok(Event::Key(key))) = maybe_event {
                        let action = input::handle_key_event(key, &mut self.tui_state);
                        match action {
                            Action::Quit => {
                                self.tui_state.should_quit = true;
                                break;
                            }
                            Action::ScrollUp | Action::ScrollDown => {
                                self.tui_state.auto_scroll = false;
                                self.tui_state.scroll_offset = match action {
                                    Action::ScrollUp => self.tui_state.scroll_offset.saturating_sub(3),
                                    _ => self.tui_state.scroll_offset.saturating_add(3),
                                };
                            }
                            Action::PageUp | Action::PageDown => {
                                self.tui_state.auto_scroll = false;
                                if let Ok(size) = terminal.size() {
                                    let page = size.height / 2;
                                    self.tui_state.scroll_offset = if matches!(action, Action::PageUp) {
                                        self.tui_state.scroll_offset.saturating_sub(page)
                                    } else {
                                        self.tui_state.scroll_offset.saturating_add(page)
                                    };
                                }
                            }
                            _ => {}
                        }
                    }
                }

                maybe_chunk = stream.next() => {
                    match maybe_chunk {
                        Some(Ok(StreamChunk::TextDelta(text))) => {
                            think_splitter.push(&text);
                            self.tui_state.streaming_response = Some(think_splitter.visible().to_string());
                            self.tui_state.status_message = Some(
                                if think_splitter.is_thinking() { "Thinking..." } else { "Receiving..." }.to_string()
                            );
                            if self.tui_state.auto_scroll { self.tui_state.scroll_offset = u16::MAX; }
                        }
                        Some(Ok(StreamChunk::ToolUse { id, name, input })) => {
                            let tool_call_id = Uuid::new_v4();
                            tool_use_records.push(ToolUseRecord { tool_call_id, api_id: id, name, input });
                        }
                        Some(Ok(StreamChunk::Done { output_tokens: ot, stop_reason: sr })) => {
                            output_tokens = ot;
                            stop_reason = sr;
                            break;
                        }
                        Some(Ok(StreamChunk::Error(e))) => {
                            self.tui_state.status_message = Some(format!("Error: {e}"));
                            break;
                        }
                        Some(Err(e)) => {
                            self.tui_state.status_message = Some(format!("Error: {e}"));
                            break;
                        }
                        None => break,
                    }
                }
            }
            terminal.draw(|frame| ui::draw(frame, &self.graph, &mut self.tui_state))?;
        }

        let (clean_response, think_content) = think_splitter.finish();
        Ok(StreamResult {
            response: clean_response,
            think_text: think_content,
            output_tokens,
            tool_use_records,
            stop_reason,
        })
    }
}
