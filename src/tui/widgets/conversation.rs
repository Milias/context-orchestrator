use crate::graph::{ConversationGraph, EdgeKind, Node, Role};
use crate::tui::widgets::display_helpers::{compute_styled_height, display_content};
use crate::tui::widgets::markdown::render_markdown;
use crate::tui::widgets::message_style::{render_agent_activity, render_message, render_streaming};
use crate::tui::widgets::trigger_highlight::highlight_triggers;
use crate::tui::{AgentVisualPhase, CachedRender, TuiState, CURSOR_FRAMES};
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

    let in_progress_ids: Vec<uuid::Uuid> = tui_state
        .agent_display
        .as_ref()
        .map_or_else(Vec::new, |d| d.iteration_node_ids.clone());

    for node in history
        .iter()
        .filter(|n| !matches!(n, Node::ThinkBlock { .. }))
    {
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
            if content.is_empty()
                && !graph
                    .sources_by_edge(node.id(), EdgeKind::Invoked)
                    .is_empty()
            {
                push_tool_indicators(node.id(), graph, tui_state, msg_content_width, &mut entries);
                continue;
            }
        }

        push_node_entry(node, graph, tui_state, msg_content_width, &mut entries);

        if matches!(
            node,
            Node::Message {
                role: Role::Assistant,
                ..
            }
        ) {
            push_tool_indicators(node.id(), graph, tui_state, msg_content_width, &mut entries);
        }
    }

    if let Some(ref display) = tui_state.agent_display {
        append_agent_display(
            display,
            graph,
            tui_state.status_message.as_ref(),
            msg_content_width,
            &mut entries,
        );
    }

    entries
}

fn push_tool_indicators<'a>(
    assistant_id: uuid::Uuid,
    graph: &'a ConversationGraph,
    tui_state: &mut TuiState,
    msg_content_width: usize,
    entries: &mut Vec<MessageEntry<'a>>,
) {
    let tool_call_ids = graph.sources_by_edge(assistant_id, EdgeKind::Invoked);
    for tc_id in &tool_call_ids {
        if let Some(tc_node) = graph.node(*tc_id) {
            push_node_entry(tc_node, graph, tui_state, msg_content_width, entries);
            let result_ids = graph.sources_by_edge(*tc_id, EdgeKind::Produced);
            for r_id in &result_ids {
                if let Some(r_node) = graph.node(*r_id) {
                    push_node_entry(r_node, graph, tui_state, msg_content_width, entries);
                }
            }
        }
    }
}

fn append_agent_display(
    display: &crate::tui::AgentDisplayState,
    graph: &ConversationGraph,
    status_message: Option<&String>,
    msg_content_width: usize,
    entries: &mut Vec<MessageEntry<'_>>,
) {
    let accumulated = display.accumulated_text(graph);
    match &display.phase {
        AgentVisualPhase::Preparing => {
            let status = status_message.map_or("Preparing...", String::as_str);
            let spinner = display.spinner_char();
            let styled = Text::from(Line::from(vec![
                Span::styled(format!("{spinner} "), Style::default().fg(Color::Green)),
                Span::styled(status.to_string(), Style::default().fg(Color::DarkGray)),
            ]));
            entries.push(MessageEntry::AgentActivity {
                height: 1,
                styled_text: styled,
            });
        }
        AgentVisualPhase::Streaming { text, is_thinking } => {
            let full_text = combine_text(&accumulated, text);
            let mut styled = render_markdown(&full_text);
            if *is_thinking && text.is_empty() {
                let spinner = display.spinner_char();
                styled.lines.push(Line::styled(
                    format!("{spinner} Thinking..."),
                    Style::default().fg(Color::DarkGray).italic(),
                ));
            }
            append_cursor(&mut styled, display.spinner_tick);
            let height = compute_styled_height(&styled, msg_content_width, false);
            entries.push(MessageEntry::Streaming {
                styled_text: styled,
                height,
            });
        }
        AgentVisualPhase::ExecutingTools => {
            let tool_lines = build_tool_status_lines(display, graph);
            let full_text = combine_text(&accumulated, &tool_lines);
            let mut styled = render_markdown(&full_text);
            append_cursor(&mut styled, display.spinner_tick);
            let height = compute_styled_height(&styled, msg_content_width, false);
            entries.push(MessageEntry::Streaming {
                styled_text: styled,
                height,
            });
        }
    }
}

/// Build status lines for running/completed tool calls from the graph.
fn build_tool_status_lines(
    display: &crate::tui::AgentDisplayState,
    graph: &ConversationGraph,
) -> String {
    use crate::graph::tool_types::ToolCallStatus;

    let mut lines = Vec::new();
    for assistant_id in &display.iteration_node_ids {
        for tc_id in graph.sources_by_edge(*assistant_id, EdgeKind::Invoked) {
            if let Some(Node::ToolCall {
                status, arguments, ..
            }) = graph.node(tc_id)
            {
                let icon = match status {
                    ToolCallStatus::Running | ToolCallStatus::Pending => display.spinner_char(),
                    ToolCallStatus::Completed => "✓",
                    ToolCallStatus::Failed => "✗",
                    ToolCallStatus::Cancelled => "⊘",
                };
                lines.push(format!("{icon} {}", arguments.display_summary()));
            }
        }
    }
    lines.join("\n")
}

fn combine_text(accumulated: &str, current: &str) -> String {
    match (accumulated.is_empty(), current.is_empty()) {
        (true, _) => current.to_string(),
        (_, true) => accumulated.to_string(),
        _ => format!("{accumulated}\n\n{current}"),
    }
}

fn append_cursor(styled: &mut Text<'static>, tick: usize) {
    let cursor = CURSOR_FRAMES[tick % CURSOR_FRAMES.len()];
    let span = Span::styled(cursor, Style::default().fg(Color::Green));
    if let Some(last_line) = styled.lines.last_mut() {
        last_line.spans.push(span);
    } else {
        styled.lines.push(Line::from(span));
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
    let valid = tui_state
        .render_cache
        .get(&id)
        .is_some_and(|c| c.cached_width == msg_content_width);
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
