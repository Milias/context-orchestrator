use crate::graph::{Node, Role};
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};

pub fn role_label(node: &Node) -> &'static str {
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

pub fn role_color(node: &Node) -> Color {
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

pub fn metadata_string(node: &Node) -> String {
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

/// Shared paragraph rendering with block, wrap, scroll, and clipping.
fn render_block_paragraph(
    frame: &mut Frame,
    area: Rect,
    block: Block<'static>,
    content: Text<'static>,
    clip_top: u16,
    full_height: u16,
) {
    let paragraph = Paragraph::new(content)
        .block(block)
        .wrap(Wrap { trim: false })
        .scroll((clip_top, 0));
    let render_area = Rect::new(area.x, area.y, area.width, area.height.min(full_height));
    frame.render_widget(paragraph, render_area);
}

pub fn render_message(
    frame: &mut Frame,
    area: Rect,
    node: &Node,
    styled_text: &Text<'static>,
    clip_top: u16,
    full_height: u16,
    has_thinking: bool,
) {
    let block = build_block(role_label(node), &metadata_string(node), role_color(node));
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
    render_block_paragraph(frame, area, block, content, clip_top, full_height);
}

pub fn render_streaming(
    frame: &mut Frame,
    area: Rect,
    styled_text: &Text<'static>,
    clip_top: u16,
    full_height: u16,
) {
    let block = build_block("assistant", "", Color::Green);
    render_block_paragraph(
        frame,
        area,
        block,
        styled_text.clone(),
        clip_top,
        full_height,
    );
}

/// Render a borderless spinner line for agent preparation phases.
pub fn render_agent_activity(frame: &mut Frame, area: Rect, styled_text: &Text<'static>) {
    let paragraph = Paragraph::new(styled_text.clone()).wrap(Wrap { trim: false });
    frame.render_widget(paragraph, area);
}
