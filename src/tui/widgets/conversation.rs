use crate::graph::{ConversationGraph, EdgeKind, Node, Role};
use crate::tui::widgets::display_helpers::{
    apply_reveal_fade, compute_styled_height, display_content, format_scroll_indicator,
};
use crate::tui::widgets::markdown::render_markdown;
use crate::tui::widgets::message_style::{render_message, render_streaming, MessageRenderParams};
use crate::tui::widgets::trigger_highlight::highlight_triggers;
use crate::tui::{AgentVisualPhase, CachedRender, TuiState, CURSOR_FRAMES};
use chrono::{DateTime, Utc};
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
        // Accumulate in u32 to avoid u16 overflow on very long conversations,
        // then clamp to u16 for scroll offset arithmetic.
        let total_height_u32: u32 = entries.iter().map(|e| e.height() as u32).sum();
        let total_height: u16 = total_height_u32.min(u32::from(u16::MAX)) as u16;
        let max_scroll = total_height.saturating_sub(inner.height);
        // Publish max_scroll so handle_scroll can clamp immediately.
        tui_state.max_scroll = max_scroll;
        tui_state.scroll.apply_max(max_scroll);
        if tui_state.scroll_mode == crate::tui::ScrollMode::Auto {
            tui_state.scroll.snap(max_scroll);
        }

        let scroll_indicator = format_scroll_indicator(
            tui_state.scroll.position(),
            max_scroll,
            tui_state.scroll_mode,
        );
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
    let scroll = i32::from(tui_state.scroll.position());
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
                prev_created_at,
                cache_key,
                height,
                has_thinking,
            } => {
                let styled_text = &tui_state.render_cache[cache_key].styled_text;
                let params = MessageRenderParams {
                    prev_created_at: *prev_created_at,
                    clip_top,
                    full_height: *height as u16,
                    has_thinking: *has_thinking,
                    is_truncated: node.is_truncated(),
                };
                render_message(frame, msg_area, node, styled_text, &params);
            }
            MessageEntry::Streaming {
                styled_text,
                height,
            } => {
                render_streaming(frame, msg_area, styled_text, clip_top, *height as u16);
            }
        }

        y_offset += h;
    }
}

enum MessageEntry<'a> {
    Node {
        node: &'a Node,
        prev_created_at: Option<DateTime<Utc>>,
        cache_key: uuid::Uuid,
        height: usize,
        has_thinking: bool,
    },
    Streaming {
        styled_text: Text<'static>,
        height: usize,
    },
}

impl MessageEntry<'_> {
    fn height(&self) -> usize {
        match self {
            Self::Node { height, .. } | Self::Streaming { height, .. } => *height,
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
    let mut last_user_created_at: Option<DateTime<Utc>> = None;
    let spinner_tick = tui_state
        .agent_display
        .as_ref()
        .map_or(0, |d| d.spinner_tick);
    let expanded = tui_state.tool_display.is_expanded();

    for node in history
        .iter()
        .filter(|n| !matches!(n, Node::ThinkBlock { .. }))
    {
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
                push_tool_status(
                    node.id(),
                    graph,
                    spinner_tick,
                    expanded,
                    msg_content_width,
                    &mut entries,
                );
                continue;
            }
        }

        let prev = if matches!(
            node,
            Node::Message {
                role: Role::Assistant,
                ..
            }
        ) {
            last_user_created_at
        } else {
            None
        };
        push_node_entry(
            node,
            prev,
            graph,
            tui_state,
            msg_content_width,
            &mut entries,
        );

        if matches!(
            node,
            Node::Message {
                role: Role::User,
                ..
            }
        ) {
            last_user_created_at = Some(node.created_at());
        }

        if matches!(
            node,
            Node::Message {
                role: Role::Assistant,
                ..
            }
        ) {
            push_tool_status(
                node.id(),
                graph,
                spinner_tick,
                expanded,
                msg_content_width,
                &mut entries,
            );
        }
    }

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

/// Render compact tool status lines for an assistant message's tool calls.
fn push_tool_status(
    assistant_id: uuid::Uuid,
    graph: &ConversationGraph,
    spinner_tick: usize,
    expanded: bool,
    msg_content_width: usize,
    entries: &mut Vec<MessageEntry<'_>>,
) {
    let lines = super::tool_status::build_tool_lines(
        graph,
        assistant_id,
        spinner_tick,
        msg_content_width,
        expanded,
    );
    if lines.is_empty() {
        return;
    }
    let styled = Text::from(lines);
    let height = compute_styled_height(&styled, msg_content_width, false);
    entries.push(MessageEntry::Streaming {
        styled_text: styled,
        height,
    });
}

fn append_agent_display(
    display: &crate::tui::AgentDisplayState,
    status_message: Option<&String>,
    msg_content_width: usize,
    entries: &mut Vec<MessageEntry<'_>>,
) {
    match &display.phase {
        AgentVisualPhase::Preparing | AgentVisualPhase::ExecutingTools => {
            let status = status_message.map_or("Preparing...", String::as_str);
            let spinner = display.spinner_char();
            let styled = Text::from(Line::from(vec![
                Span::styled(format!("{spinner} "), Style::default().fg(Color::Green)),
                Span::styled(status.to_string(), Style::default().fg(Color::DarkGray)),
            ]));
            let height = compute_styled_height(&styled, msg_content_width, false);
            entries.push(MessageEntry::Streaming {
                styled_text: styled,
                height,
            });
        }
        AgentVisualPhase::Streaming { text, is_thinking } => {
            // Slice text at the revealed character boundary
            let total_chars = text.chars().count();
            let reveal_count = display.revealed_chars.min(total_chars);
            let byte_offset = text
                .char_indices()
                .nth(reveal_count)
                .map_or(text.len(), |(i, _)| i);
            let revealed = &text[..byte_offset];

            let mut styled = render_markdown(revealed);

            // Apply fade-in gradient when there are unrevealed characters
            if reveal_count < total_chars {
                apply_reveal_fade(&mut styled, 8);
            }

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
    prev_created_at: Option<DateTime<Utc>>,
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
        prev_created_at,
        cache_key: id,
        height: c.height,
        has_thinking: c.has_thinking,
    });
}
