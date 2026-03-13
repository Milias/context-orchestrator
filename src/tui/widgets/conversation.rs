use crate::graph::{ConversationGraph, EdgeKind, Node, Role};
use crate::tui::widgets::display_helpers::{compute_styled_height, display_content};
use crate::tui::widgets::markdown::render_markdown;
use crate::tui::widgets::message_style::{render_agent_activity, render_message, render_streaming};
use crate::tui::widgets::trigger_highlight::highlight_triggers;
use crate::tui::{AgentVisualPhase, CachedRender, TuiState};
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders};

pub fn render(frame: &mut Frame, area: Rect, graph: &ConversationGraph, tui_state: &mut TuiState) {
    let history = graph
        .get_branch_history(graph.active_branch())
        .unwrap_or_default();

    let probe_block = Block::default().borders(Borders::ALL);
    let inner = probe_block.inner(area);

    if inner.width < 4 || inner.height == 0 {
        let outer_block = Block::default().title("Conversation").borders(Borders::ALL);
        frame.render_widget(outer_block, area);
        return;
    }

    let msg_content_width = inner.width.saturating_sub(2) as usize;
    if msg_content_width == 0 {
        return;
    }

    let entries = build_entries(&history, graph, &mut *tui_state, msg_content_width);

    // All values in this scroll math are bounded by terminal dimensions (u16) and message
    // counts, so casts between i32/u16/usize cannot overflow in practice.
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_possible_wrap,
        clippy::cast_sign_loss,
        clippy::cast_lossless
    )]
    {
        let total_height: u16 = entries.iter().map(|e| e.height() as u16).sum();
        let max_scroll = total_height.saturating_sub(inner.height);
        if tui_state.auto_scroll {
            tui_state.scroll_offset = max_scroll;
        } else {
            tui_state.scroll_offset = tui_state.scroll_offset.min(max_scroll);
        }

        let scroll_indicator = format_scroll_indicator(tui_state.scroll_offset, max_scroll);
        let mut outer_block = Block::default().title("Conversation").borders(Borders::ALL);
        if !scroll_indicator.is_empty() {
            outer_block = outer_block.title(
                Line::styled(scroll_indicator, Style::default().fg(Color::DarkGray))
                    .alignment(Alignment::Right),
            );
        }
        frame.render_widget(outer_block, area);

        render_entries(frame, &entries, tui_state, inner);
    }
}

// Cast safety: bounded by terminal dimensions (u16) and message counts.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::cast_lossless
)]
fn render_entries(
    frame: &mut Frame,
    entries: &[MessageEntry<'_>],
    tui_state: &TuiState,
    inner: Rect,
) {
    let scroll = i32::from(tui_state.scroll_offset);
    let viewport_h = i32::from(inner.height);
    let mut y_offset: i32 = -scroll;

    for entry in entries {
        let h = entry.height() as i32;
        if y_offset + h <= 0 {
            y_offset += h;
            continue;
        }
        if y_offset >= viewport_h {
            break;
        }

        let clip_top = (-y_offset).max(0) as u16;
        let visible_y = y_offset.max(0) as u16;
        let visible_h = (h - i32::from(clip_top)).min(viewport_h - i32::from(visible_y)) as u16;
        if visible_h == 0 {
            y_offset += h;
            continue;
        }

        let msg_area = Rect::new(inner.x, inner.y + visible_y, inner.width, visible_h);

        match entry {
            MessageEntry::Node {
                node,
                cache_key,
                height,
                has_thinking,
            } => {
                let styled_text = &tui_state.render_cache[cache_key].styled_text;
                render_message(
                    frame,
                    msg_area,
                    node,
                    styled_text,
                    clip_top,
                    *height as u16,
                    *has_thinking,
                );
            }
            MessageEntry::Streaming {
                styled_text,
                height,
            } => {
                render_streaming(frame, msg_area, styled_text, clip_top, *height as u16);
            }
            MessageEntry::AgentActivity { styled_text, .. } => {
                render_agent_activity(frame, msg_area, styled_text);
            }
        }

        y_offset += h;
    }
}

enum MessageEntry<'a> {
    Node {
        node: &'a Node,
        cache_key: uuid::Uuid,
        height: usize,
        has_thinking: bool,
    },
    Streaming {
        styled_text: Text<'static>,
        height: usize,
    },
    AgentActivity {
        styled_text: Text<'static>,
        height: usize,
    },
}

impl MessageEntry<'_> {
    fn height(&self) -> usize {
        match self {
            Self::Node { height, .. }
            | Self::Streaming { height, .. }
            | Self::AgentActivity { height, .. } => *height,
        }
    }
}

fn build_entries<'a>(
    history: &[&'a Node],
    graph: &'a ConversationGraph,
    tui_state: &mut TuiState,
    msg_content_width: usize,
) -> Vec<MessageEntry<'a>> {
    let mut entries: Vec<MessageEntry<'a>> = Vec::new();

    // IDs being rendered by the agent display (suppress from history)
    let in_progress_ids: Vec<uuid::Uuid> = tui_state
        .agent_display
        .as_ref()
        .map_or_else(Vec::new, |d| d.iteration_node_ids.clone());

    for node in history
        .iter()
        .filter(|n| !matches!(n, Node::ThinkBlock { .. }))
    {
        // Skip nodes currently owned by the agent display
        if in_progress_ids.contains(&node.id()) {
            continue;
        }

        // Skip empty assistant messages that have tool call children
        if let Node::Message {
            role: Role::Assistant,
            content,
            ..
        } = node
        {
            if content.is_empty() {
                let has_tools = !graph
                    .sources_by_edge(node.id(), EdgeKind::Invoked)
                    .is_empty();
                if has_tools {
                    // Still render the tool calls inline
                    push_tool_indicators(node.id(), graph, msg_content_width, &mut entries);
                    continue;
                }
            }
        }

        push_node_entry(node, graph, tui_state, msg_content_width, &mut entries);

        // After an assistant message, inject its tool calls and results
        if matches!(
            node,
            Node::Message {
                role: Role::Assistant,
                ..
            }
        ) {
            push_tool_indicators(node.id(), graph, msg_content_width, &mut entries);
        }
    }

    // Append agent display state at the bottom
    if let Some(ref display) = tui_state.agent_display {
        append_agent_display(
            display,
            tui_state.status_message.as_ref(),
            msg_content_width,
            &mut entries,
        );
    }

    entries
}

/// Build compact tool call indicator entries for an assistant message's tool calls.
fn push_tool_indicators<'a>(
    assistant_id: uuid::Uuid,
    graph: &'a ConversationGraph,
    msg_content_width: usize,
    entries: &mut Vec<MessageEntry<'a>>,
) {
    let tool_call_ids = graph.sources_by_edge(assistant_id, EdgeKind::Invoked);
    for tc_id in &tool_call_ids {
        if let Some(tc_node) = graph.node(*tc_id) {
            push_node_entry_raw(tc_node, graph, msg_content_width, entries);
            let result_ids = graph.sources_by_edge(*tc_id, EdgeKind::Produced);
            for r_id in &result_ids {
                if let Some(r_node) = graph.node(*r_id) {
                    push_node_entry_raw(r_node, graph, msg_content_width, entries);
                }
            }
        }
    }
}

/// Push a node entry using a temporary cache (no `TuiState` needed).
fn push_node_entry_raw<'a>(
    node: &'a Node,
    graph: &'a ConversationGraph,
    msg_content_width: usize,
    entries: &mut Vec<MessageEntry<'a>>,
) {
    // For tool indicators rendered outside the cache, compute inline.
    // These are cheap since tool call summaries are short.
    let content = display_content(node, graph);
    let styled = render_markdown(&content);
    let has_thinking = false;
    let height = compute_styled_height(&styled, msg_content_width, has_thinking);

    // We can't easily cache these without TuiState, so we use a raw entry
    // that re-renders each frame. Tool summaries are small, so this is fine.
    entries.push(MessageEntry::Streaming {
        styled_text: styled,
        height,
    });
}

/// Append the agent display (spinner, streaming, or tool execution) at the bottom.
fn append_agent_display(
    display: &crate::tui::AgentDisplayState,
    status_message: Option<&String>,
    msg_content_width: usize,
    entries: &mut Vec<MessageEntry<'_>>,
) {
    match &display.phase {
        AgentVisualPhase::Preparing => {
            let status = status_message.map_or("Preparing...", String::as_str);
            let spinner = display.spinner_char();
            let line = Line::from(vec![
                Span::styled(format!("{spinner} "), Style::default().fg(Color::Green)),
                Span::styled(status.to_string(), Style::default().fg(Color::DarkGray)),
            ]);
            let styled = Text::from(line);
            entries.push(MessageEntry::AgentActivity {
                height: 1,
                styled_text: styled,
            });
        }
        AgentVisualPhase::Streaming { text, is_thinking } => {
            let full_text = build_accumulated_streaming_text(&display.accumulated_text, text);
            let mut styled = render_markdown(&full_text);
            if *is_thinking && text.is_empty() {
                let spinner = display.spinner_char();
                styled.lines.push(Line::styled(
                    format!("{spinner} Thinking..."),
                    Style::default().fg(Color::DarkGray).italic(),
                ));
            }
            append_cursor(&mut styled);
            let height = compute_styled_height(&styled, msg_content_width, false);
            entries.push(MessageEntry::Streaming {
                styled_text: styled,
                height,
            });
        }
        AgentVisualPhase::ExecutingTools { tool_count } => {
            let spinner = display.spinner_char();
            let tool_line = format!("{spinner} Executing {tool_count} tool call(s)...");
            let full_text = if display.accumulated_text.is_empty() {
                tool_line
            } else {
                format!("{}\n\n{tool_line}", display.accumulated_text)
            };
            let mut styled = render_markdown(&full_text);
            append_cursor(&mut styled);
            let height = compute_styled_height(&styled, msg_content_width, false);
            entries.push(MessageEntry::Streaming {
                styled_text: styled,
                height,
            });
        }
    }
}

fn build_accumulated_streaming_text(accumulated: &str, current: &str) -> String {
    if accumulated.is_empty() {
        current.to_string()
    } else if current.is_empty() {
        accumulated.to_string()
    } else {
        format!("{accumulated}\n\n{current}")
    }
}

fn append_cursor(styled: &mut Text<'static>) {
    if let Some(last_line) = styled.lines.last_mut() {
        last_line
            .spans
            .push(Span::styled("▌", Style::default().fg(Color::Green)));
    } else {
        styled.lines.push(Line::from(Span::styled(
            "▌",
            Style::default().fg(Color::Green),
        )));
    }
}

fn push_node_entry<'a>(
    node: &'a Node,
    graph: &ConversationGraph,
    tui_state: &mut TuiState,
    msg_content_width: usize,
    entries: &mut Vec<MessageEntry<'a>>,
) {
    let id = node.id();
    let cached = tui_state.render_cache.get(&id);
    let valid = cached.is_some_and(|c| c.cached_width == msg_content_width);
    if !valid {
        let content = display_content(node, graph);
        let mut styled = render_markdown(&content);
        if matches!(
            node,
            Node::Message {
                role: Role::User,
                ..
            }
        ) {
            highlight_triggers(&mut styled);
        }
        let has_thinking = graph.has_think_block(id);
        let height = compute_styled_height(&styled, msg_content_width, has_thinking);
        tui_state.render_cache.insert(
            id,
            CachedRender {
                styled_text: styled,
                height,
                has_thinking,
                cached_width: msg_content_width,
            },
        );
    }
    let c = &tui_state.render_cache[&id];
    entries.push(MessageEntry::Node {
        node,
        cache_key: id,
        height: c.height,
        has_thinking: c.has_thinking,
    });
}

fn format_scroll_indicator(offset: u16, max: u16) -> String {
    match () {
        () if max == 0 => String::new(),
        () if offset >= max => " [END] ".to_string(),
        () => format!(" [{}%] ", (u32::from(offset) * 100) / u32::from(max)),
    }
}
