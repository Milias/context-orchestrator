use crate::graph::{ConversationGraph, Node, Role};
use crate::tui::TuiState;
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

    // Build list of renderable messages with their heights
    let mut entries: Vec<MessageEntry> = history
        .iter()
        .map(|node| {
            let height = compute_height(node.content(), msg_content_width);
            MessageEntry::Node { node, height }
        })
        .collect();

    if let Some(ref streaming) = tui_state.streaming_response {
        // +1 line for the cursor
        let text_with_cursor = format!("{streaming}▌");
        let height = compute_height(&text_with_cursor, msg_content_width);
        entries.push(MessageEntry::Streaming {
            content: text_with_cursor,
            height,
        });
    }

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
                MessageEntry::Node { node, height } => {
                    render_message(frame, msg_area, node, clip_top, *height as u16);
                }
                MessageEntry::Streaming { content, height } => {
                    render_streaming(frame, msg_area, content, clip_top, *height as u16);
                }
            }

            y_offset += h;
        }
    }
}

enum MessageEntry<'a> {
    Node { node: &'a Node, height: usize },
    Streaming { content: String, height: usize },
}

impl MessageEntry<'_> {
    fn height(&self) -> usize {
        match self {
            MessageEntry::Node { height, .. } | MessageEntry::Streaming { height, .. } => *height,
        }
    }
}

fn compute_height(content: &str, content_width: usize) -> usize {
    let mut lines = 0usize;
    for line in content.lines() {
        let char_count = line.chars().count();
        if char_count == 0 {
            lines += 1;
        } else {
            lines += char_count.div_ceil(content_width);
        }
    }
    if content.is_empty() {
        lines = 1;
    }
    lines + 2 // +2 for top/bottom border
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
        Node::Tool { .. } => Color::Magenta,
        Node::SystemDirective { .. } | Node::BackgroundTask { .. } => Color::DarkGray,
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

fn render_message(frame: &mut Frame, area: Rect, node: &Node, clip_top: u16, full_height: u16) {
    let label = role_label(node);
    let color = role_color(node);
    let metadata = metadata_string(node);
    let block = build_block(label, &metadata, color);

    let paragraph = Paragraph::new(node.content().to_string())
        .block(block)
        .wrap(Wrap { trim: false })
        .scroll((clip_top, 0));

    // Render into a virtual area of full height, clipped to the visible rect
    let virtual_area = Rect::new(area.x, area.y, area.width, full_height);
    // Use the actual area for rendering (frame clips automatically)
    let render_area = Rect::new(
        area.x,
        area.y,
        area.width,
        area.height.min(virtual_area.height),
    );
    frame.render_widget(paragraph, render_area);
}

fn render_streaming(frame: &mut Frame, area: Rect, content: &str, clip_top: u16, full_height: u16) {
    let block = build_block("assistant", "", Color::Green);

    let paragraph = Paragraph::new(content.to_string())
        .block(block)
        .wrap(Wrap { trim: false })
        .scroll((clip_top, 0));

    let render_area = Rect::new(area.x, area.y, area.width, area.height.min(full_height));
    frame.render_widget(paragraph, render_area);
}
