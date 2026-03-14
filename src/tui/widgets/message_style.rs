use crate::graph::{Node, Role};
use chrono::{DateTime, Utc};
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
        Node::Question { .. } => "question",
        Node::Answer { .. } => "answer",
        Node::ApiError { .. } => "error",
    }
}

pub fn role_color(node: &Node) -> Color {
    match node {
        Node::Message { role, .. } => match role {
            Role::User => Color::Cyan,
            Role::Assistant => Color::Green,
            Role::System => Color::DarkGray,
        },
        Node::WorkItem { .. } | Node::Question { .. } => Color::Yellow,
        Node::GitFile { .. } => Color::Blue,
        Node::Tool { .. } | Node::ToolCall { .. } => Color::Magenta,
        Node::ToolResult { .. } => Color::Cyan,
        Node::Answer { .. } => Color::Green,
        // ThinkBlock nodes are filtered out before rendering; arm is required for exhaustiveness.
        Node::ApiError { .. } => Color::Red,
        Node::SystemDirective { .. } | Node::BackgroundTask { .. } | Node::ThinkBlock { .. } => {
            Color::DarkGray
        }
    }
}

pub fn metadata_string(node: &Node, prev_created_at: Option<DateTime<Utc>>) -> String {
    let mut parts: Vec<String> = Vec::new();

    // Model name for assistant messages
    if let Some(model) = node.model() {
        parts.push(short_model_name(model).to_string());
    }

    // Token counts
    match (node.input_tokens(), node.output_tokens()) {
        (Some(i), Some(o)) => parts.push(format!("{i}in/{o}out")),
        (Some(t), None) | (None, Some(t)) => parts.push(format!("{t} tok")),
        _ => {}
    }

    // Elapsed time (user → assistant)
    if matches!(
        node,
        Node::Message {
            role: Role::Assistant,
            ..
        }
    ) {
        if let Some(prev) = prev_created_at {
            let dur = node.created_at() - prev;
            if dur.num_milliseconds() > 0 {
                parts.push(format_duration(dur));
            }
        }
    }

    // Tool call duration
    if let Node::ToolCall {
        created_at,
        completed_at: Some(completed),
        ..
    } = node
    {
        let dur = *completed - *created_at;
        if dur.num_milliseconds() > 0 {
            parts.push(format_duration(dur));
        }
    }

    parts.join(" | ")
}

fn short_model_name(model: &str) -> &str {
    model.strip_prefix("claude-").unwrap_or(model)
}

fn format_duration(dur: chrono::TimeDelta) -> String {
    let total_ms = dur.num_milliseconds();
    if total_ms < 1000 {
        return format!("{total_ms}ms");
    }
    let secs = dur.num_seconds();
    if secs < 60 {
        return format!("{secs}.{}s", (total_ms % 1000) / 100);
    }
    format!("{}m {}s", secs / 60, secs % 60)
}

fn timestamp_string(node: &Node) -> String {
    node.created_at()
        .with_timezone(&chrono::Local)
        .format("%H:%M")
        .to_string()
}

fn build_block(
    label: &str,
    metadata: &str,
    timestamp: &str,
    border_width: u16,
    color: Color,
) -> Block<'static> {
    let mut block = Block::default()
        .title(Line::styled(
            format!(" {label} "),
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(color));

    // Only show metadata if it fits without overlapping the label
    let fits = metadata.len() + label.len() + 6 <= border_width as usize;
    if !metadata.is_empty() && fits {
        block = block.title(
            Line::styled(
                format!(" {metadata} "),
                Style::default().fg(Color::DarkGray),
            )
            .alignment(Alignment::Right),
        );
    }

    if !timestamp.is_empty() {
        block = block.title_bottom(Line::styled(
            format!(" {timestamp} "),
            Style::default().fg(Color::DarkGray),
        ));
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

pub struct MessageRenderParams {
    pub prev_created_at: Option<DateTime<Utc>>,
    pub clip_top: u16,
    pub full_height: u16,
    pub has_thinking: bool,
    pub is_truncated: bool,
}

pub fn render_message(
    frame: &mut Frame,
    area: Rect,
    node: &Node,
    styled_text: &Text<'static>,
    params: &MessageRenderParams,
) {
    let metadata = metadata_string(node, params.prev_created_at);
    let timestamp = timestamp_string(node);
    let mut block = build_block(
        role_label(node),
        &metadata,
        &timestamp,
        area.width,
        role_color(node),
    );
    if params.is_truncated {
        block = block.title_bottom(
            Line::styled(" truncated ", Style::default().fg(Color::Yellow))
                .alignment(Alignment::Right),
        );
    }
    let mut content = styled_text.clone();
    if params.has_thinking {
        content.lines.insert(
            0,
            Line::styled(
                "[thinking...]",
                Style::default().fg(Color::DarkGray).italic(),
            ),
        );
    }
    render_block_paragraph(
        frame,
        area,
        block,
        content,
        params.clip_top,
        params.full_height,
    );
}

pub fn render_streaming(
    frame: &mut Frame,
    area: Rect,
    styled_text: &Text<'static>,
    clip_top: u16,
    full_height: u16,
) {
    let now = chrono::Local::now().format("%H:%M").to_string();
    let block = build_block("assistant", "", &now, area.width, Color::Green);
    render_block_paragraph(
        frame,
        area,
        block,
        styled_text.clone(),
        clip_top,
        full_height,
    );
}
