use crate::llm::error::ApiError;
use crate::llm::retry::RetryConfig;
use crate::llm::{ChatConfig, ChatMessage, StreamChunk};
use crate::tui::input::{self, Action};
use crate::tui::ui;

use super::think_splitter::ThinkSplitter;
use super::App;

use crossterm::event::{Event, EventStream, KeyCode, KeyEventKind};
use futures::stream::Stream;
use futures::StreamExt;
use ratatui::prelude::*;
use std::io;
use std::pin::Pin;
use std::time::{Duration, Instant};
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

const MAX_STREAM_RETRIES: u32 = 2;

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
        self.tui_state.error_message = None;
        self.tui_state.auto_scroll = true;
        self.tui_state.scroll_offset = u16::MAX;
        terminal.draw(|frame| ui::draw(frame, &self.graph, &mut self.tui_state))?;

        let Some(mut stream) = self
            .try_connect_chat(&messages, config, terminal, event_stream, "Connecting")
            .await?
        else {
            self.tui_state.streaming_response = None;
            return Ok(StreamResult::default());
        };

        self.tui_state.error_message = None;
        self.consume_stream(&mut stream, &messages, config, terminal, event_stream)
            .await
    }

    async fn consume_stream(
        &mut self,
        stream: &mut Pin<Box<dyn Stream<Item = anyhow::Result<StreamChunk>> + Send>>,
        messages: &[ChatMessage],
        config: &ChatConfig,
        terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
        event_stream: &mut EventStream,
    ) -> anyhow::Result<StreamResult> {
        let mut think_splitter = ThinkSplitter::new();
        let mut output_tokens = None;
        let mut tool_use_records = Vec::new();
        let mut stop_reason = None;
        let mut stream_retries = 0u32;
        let mut last_draw = Instant::now();
        let frame_budget = Duration::from_millis(16);

        loop {
            let mut needs_draw = false;
            tokio::select! {
                biased; // event branch first so user input isn't starved by SSE bursts

                maybe_event = event_stream.next() => {
                    if let Some(Ok(Event::Key(key))) = maybe_event {
                        if key.kind != KeyEventKind::Press { continue; }
                        if self.handle_streaming_key(key, terminal) {
                            break;
                        }
                        needs_draw = true;
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
                            if last_draw.elapsed() >= frame_budget {
                                needs_draw = true;
                            }
                        }
                        Some(Ok(StreamChunk::ToolUse { id, name, input })) => {
                            let tool_call_id = Uuid::new_v4();
                            let summary = format_tool_summary(&name, &input);
                            think_splitter.push(&format!("\n\n> **{summary}**"));
                            self.tui_state.streaming_response = Some(think_splitter.visible().to_string());
                            self.tui_state.status_message = Some(format!("Tool call: {name}"));
                            needs_draw = true;
                            tool_use_records.push(ToolUseRecord { tool_call_id, api_id: id, name, input });
                        }
                        Some(Ok(StreamChunk::Done { output_tokens: ot, stop_reason: sr })) => {
                            output_tokens = ot;
                            stop_reason = sr;
                            break;
                        }
                        Some(Ok(StreamChunk::Error(e))) => {
                            self.tui_state.error_message = Some(format!("Stream error: {e}"));
                            if stream_retries < MAX_STREAM_RETRIES {
                                stream_retries += 1;
                                if let Some(new) = self.try_connect_chat(
                                    messages, config, terminal, event_stream, "Reconnecting",
                                ).await? {
                                    *stream = new;
                                    think_splitter = ThinkSplitter::new();
                                    self.tui_state.error_message = None;
                                    continue;
                                }
                            }
                            break;
                        }
                        Some(Err(e)) => {
                            let retryable = e.downcast_ref::<ApiError>()
                                .is_some_and(ApiError::is_retryable);
                            self.tui_state.error_message = Some(format_error(&e));
                            if retryable && stream_retries < MAX_STREAM_RETRIES {
                                stream_retries += 1;
                                if let Some(new) = self.try_connect_chat(
                                    messages, config, terminal, event_stream, "Reconnecting",
                                ).await? {
                                    *stream = new;
                                    think_splitter = ThinkSplitter::new();
                                    self.tui_state.error_message = None;
                                    continue;
                                }
                            }
                            break;
                        }
                        None => break,
                    }
                }
            }
            if needs_draw {
                terminal.draw(|frame| ui::draw(frame, &self.graph, &mut self.tui_state))?;
                last_draw = Instant::now();
            }
        }
        terminal.draw(|frame| ui::draw(frame, &self.graph, &mut self.tui_state))?;

        let (clean_response, think_content) = think_splitter.finish();
        self.tui_state.streaming_response = None;
        Ok(StreamResult {
            response: clean_response,
            think_text: think_content,
            output_tokens,
            tool_use_records,
            stop_reason,
        })
    }

    /// Try to establish a chat stream with retry and cancellation.
    /// Returns `Some(stream)` on success, `None` if cancelled or retries exhausted.
    /// On failure, sets `error_message` on the status bar (right side, red).
    async fn try_connect_chat(
        &mut self,
        messages: &[ChatMessage],
        config: &ChatConfig,
        terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
        event_stream: &mut EventStream,
        context_label: &str,
    ) -> anyhow::Result<Option<Pin<Box<dyn Stream<Item = anyhow::Result<StreamChunk>> + Send>>>>
    {
        let retry_config = RetryConfig::default();

        for attempt in 1..=retry_config.max_attempts {
            match self.provider.chat(messages.to_vec(), config).await {
                Ok(s) => {
                    self.tui_state.error_message = None;
                    return Ok(Some(s));
                }
                Err(e) => {
                    let retryable = e
                        .downcast_ref::<ApiError>()
                        .is_some_and(ApiError::is_retryable);

                    if !retryable || attempt == retry_config.max_attempts {
                        self.tui_state.error_message = Some(format_error(&e));
                        return Ok(None);
                    }

                    let delay = retry_config.delay_for(attempt - 1, e.downcast_ref::<ApiError>());
                    self.tui_state.status_message = Some(format!(
                        "{context_label} ({attempt}/{})...",
                        retry_config.max_attempts,
                    ));
                    self.tui_state.error_message = Some(format_error(&e));
                    terminal.draw(|frame| {
                        ui::draw(frame, &self.graph, &mut self.tui_state);
                    })?;

                    // Cancellable sleep: Esc aborts the retry loop
                    let cancelled = tokio::select! {
                        () = tokio::time::sleep(delay) => false,
                        maybe_event = event_stream.next() => {
                            match maybe_event {
                                Some(Ok(Event::Key(key))) if key.code == KeyCode::Esc => true,
                                Some(Ok(Event::Key(key))) => {
                                    self.handle_streaming_key(key, terminal);
                                    self.tui_state.should_quit
                                }
                                _ => false,
                            }
                        }
                    };
                    if cancelled {
                        return Ok(None);
                    }
                }
            }
        }

        Ok(None)
    }

    /// Handle a key event during streaming. Returns `true` if the loop should break (quit).
    fn handle_streaming_key(
        &mut self,
        key: crossterm::event::KeyEvent,
        terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    ) -> bool {
        let action = input::handle_key_event(key, &mut self.tui_state);
        match action {
            Action::Quit => {
                self.tui_state.should_quit = true;
                return true;
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
        false
    }
}

fn format_error(e: &anyhow::Error) -> String {
    e.downcast_ref::<ApiError>()
        .map_or_else(|| format!("{e}"), ToString::to_string)
}

/// Fields extracted from tool input JSON for display summaries.
/// Uses `Option` for all fields since each tool only provides a subset.
#[derive(serde::Deserialize)]
struct ToolInputFields {
    path: Option<String>,
    pattern: Option<String>,
    query: Option<String>,
}

/// Extract a concise one-line summary from a tool call for display during streaming.
fn format_tool_summary(name: &str, input_json: &str) -> String {
    let fields: ToolInputFields = match serde_json::from_str(input_json) {
        Ok(f) => f,
        Err(_) => return name.to_string(),
    };
    let key_value = match name {
        "read_file" | "write_file" | "list_directory" => fields.path,
        "search_files" => fields.pattern,
        "web_search" => fields.query,
        _ => None,
    };
    match key_value {
        Some(v) => format!("{name}: {v}"),
        None => name.to_string(),
    }
}
