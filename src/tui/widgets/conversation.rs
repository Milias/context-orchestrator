use crate::graph::{ConversationGraph, Node, Role};
use crate::tui::widgets::markdown::render_markdown;
use crate::tui::widgets::trigger_highlight::highlight_triggers;
use crate::tui::{CachedRender, TuiState};
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};

pub fn render(frame: &mut Frame, area: Rect, graph: &ConversationGraph, tui_state: &mut TuiState) {
    let history = graph
        .get_branch_history(graph.active_branch())
        .unwrap_or_default();

    // Compute inner area without rendering yet — we need scroll info for the title
    let probe_block = Block::default().borders(Borders::ALL);
    let inner = probe_block.inner(area);

    if inner.width < 4 || inner.height == 0 {
        let outer_block = Block::default().title("Conversation").borders(Borders::ALL);
        frame.render_widget(outer_block, area);
        return;
    }

    // Content width inside a message block (block border takes 2 cols)
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
        tui_state.scroll_offset = tui_state.scroll_offset.min(max_scroll);

        let scroll_indicator = if max_scroll == 0 {
            String::new()
        } else if tui_state.scroll_offset >= max_scroll {
            " [END] ".to_string()
        } else {
            let pct = (u32::from(tui_state.scroll_offset) * 100) / u32::from(max_scroll);
            format!(" [{pct}%] ")
        };
        let mut outer_block = Block::default().title("Conversation").borders(Borders::ALL);
        if !scroll_indicator.is_empty() {
            outer_block = outer_block.title(
                Line::styled(scroll_indicator, Style::default().fg(Color::DarkGray))
                    .alignment(Alignment::Right),
            );
        }
        frame.render_widget(outer_block, area);

        let scroll = i32::from(tui_state.scroll_offset);
        let viewport_h = i32::from(inner.height);
        let mut y_offset: i32 = -scroll;

        for entry in &entries {
            let h = entry.height() as i32;

            // Skip if entirely above viewport
            if y_offset + h <= 0 {
                y_offset += h;
                continue;
            }
            // Stop if entirely below viewport
            if y_offset >= viewport_h {
                break;
            }

            // Compute visible rect, clipping top/bottom
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
            }

            y_offset += h;
        }
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
}

impl MessageEntry<'_> {
    fn height(&self) -> usize {
        match self {
            MessageEntry::Node { height, .. } | MessageEntry::Streaming { height, .. } => *height,
        }
    }
}

fn build_entries<'a>(
    history: &[&'a Node],
    graph: &ConversationGraph,
    tui_state: &mut TuiState,
    msg_content_width: usize,
) -> Vec<MessageEntry<'a>> {
    let mut entries: Vec<MessageEntry<'a>> = history
        .iter()
        .filter(|node| !matches!(node, Node::ThinkBlock { .. }))
        .map(|node| {
            let id = node.id();
            let cached = tui_state.render_cache.get(&id);
            let valid = cached.is_some_and(|c| c.cached_width == msg_content_width);
            if !valid {
                let mut styled = render_markdown(node.content());
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
            MessageEntry::Node {
                node,
                cache_key: id,
                height: c.height,
                has_thinking: c.has_thinking,
            }
        })
        .collect();

    if let Some(ref streaming) = tui_state.streaming_response {
        let mut styled = render_markdown(streaming);
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
        let height = compute_styled_height(&styled, msg_content_width, false);
        entries.push(MessageEntry::Streaming {
            styled_text: styled,
            height,
        });
    }

    entries
}

/// Compute the rendered height of styled text within a given content width.
/// Each `Line` in the `Text` may wrap if its visible width exceeds `content_width`.
/// +2 for the message block border (top/bottom). +1 if `has_thinking` (collapsed indicator).
fn compute_styled_height(text: &Text<'_>, content_width: usize, has_thinking: bool) -> usize {
    if content_width == 0 {
        return 2;
    }
    let mut total_lines = 0usize;
    if has_thinking {
        total_lines += 1; // "[thinking...]" indicator line
    }
    for line in &text.lines {
        let w = line.width();
        if w == 0 {
            total_lines += 1;
        } else {
            total_lines += w.div_ceil(content_width);
        }
    }
    if text.lines.is_empty() {
        total_lines = 1;
    }
    total_lines + 2 // +2 for top/bottom border
}

fn role_label(node: &Node) -> &'static str {
    match node {
        Node::SystemDirective { .. } => "system",
        Node::Message { role, .. } => match role {
            Role::User => "you",
            Role::Assistant => "assistant",
            Role::System => "system",
        },
        Node::WorkItem { .. } => "task",
        Node::GitFile { .. } => "file",
        Node::Tool { .. } => "tool",
        Node::BackgroundTask { .. } => "bg",
        Node::ToolCall { .. } => "call",
        Node::ToolResult { .. } => "result",
        // ThinkBlock nodes are filtered out before rendering; arm is required for exhaustiveness.
        Node::ThinkBlock { .. } => "think",
    }
}

fn role_color(node: &Node) -> Color {
    match node {
        Node::Message { role, .. } => match role {
            Role::User => Color::Cyan,
            Role::Assistant => Color::Green,
            Role::System => Color::DarkGray,
        },
        Node::WorkItem { .. } => Color::Yellow,
        Node::GitFile { .. } => Color::Blue,
        Node::Tool { .. } | Node::ToolCall { .. } => Color::Magenta,
        Node::ToolResult { .. } => Color::Cyan,
        // ThinkBlock nodes are filtered out before rendering; arm is required for exhaustiveness.
        Node::SystemDirective { .. } | Node::BackgroundTask { .. } | Node::ThinkBlock { .. } => {
            Color::DarkGray
        }
    }
}

fn metadata_string(node: &Node) -> String {
    let tokens = node.input_tokens().or(node.output_tokens());
    match tokens {
        Some(t) => format!("{t} tokens"),
        None => String::new(),
    }
}

fn build_block(label: &str, metadata: &str, color: Color) -> Block<'static> {
    let mut block = Block::default()
        .title(Line::styled(
            format!(" {label} "),
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(color));

    if !metadata.is_empty() {
        block = block.title(
            Line::styled(
                format!(" {metadata} "),
                Style::default().fg(Color::DarkGray),
            )
            .alignment(Alignment::Right),
        );
    }

    block
}

fn render_message(
    frame: &mut Frame,
    area: Rect,
    node: &Node,
    styled_text: &Text<'static>,
    clip_top: u16,
    full_height: u16,
    has_thinking: bool,
) {
    let label = role_label(node);
    let color = role_color(node);
    let metadata = metadata_string(node);
    let block = build_block(label, &metadata, color);

    let mut content = styled_text.clone();
    if has_thinking {
        content.lines.insert(
            0,
            Line::styled(
                "[thinking...]",
                Style::default().fg(Color::DarkGray).italic(),
            ),
        );
    }

    let paragraph = Paragraph::new(content)
        .block(block)
        .wrap(Wrap { trim: false })
        .scroll((clip_top, 0));

    let render_area = Rect::new(area.x, area.y, area.width, area.height.min(full_height));
    frame.render_widget(paragraph, render_area);
}

fn render_streaming(
    frame: &mut Frame,
    area: Rect,
    styled_text: &Text<'static>,
    clip_top: u16,
    full_height: u16,
) {
    let block = build_block("assistant", "", Color::Green);

    let paragraph = Paragraph::new(styled_text.clone())
        .block(block)
        .wrap(Wrap { trim: false })
        .scroll((clip_top, 0));

    let render_area = Rect::new(area.x, area.y, area.width, area.height.min(full_height));
    frame.render_widget(paragraph, render_area);
}
