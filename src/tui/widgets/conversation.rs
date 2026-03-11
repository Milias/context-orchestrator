use crate::graph::{ConversationGraph, Node, Role};
use crate::tui::TuiState;
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Paragraph};

pub fn render(frame: &mut Frame, area: Rect, graph: &ConversationGraph, tui_state: &TuiState) {
    let history = graph
        .get_branch_history(graph.active_branch())
        .unwrap_or_default();

    let outer_block = Block::default().title("Conversation").borders(Borders::ALL);
    let inner = outer_block.inner(area);
    frame.render_widget(outer_block, area);

    let box_width = inner.width as usize;
    if box_width < 6 {
        return;
    }
    let content_width = box_width.saturating_sub(4); // "│ " + content + " │"

    let mut lines: Vec<Line> = Vec::new();

    for node in &history {
        render_message_box(node, content_width, box_width, &mut lines);
    }

    if let Some(ref streaming) = tui_state.streaming_response {
        render_streaming_box(streaming, content_width, box_width, &mut lines);
    }

    let paragraph = Paragraph::new(lines).scroll((tui_state.scroll_offset, 0));
    frame.render_widget(paragraph, inner);
}

fn role_label(node: &Node) -> &'static str {
    match node {
        Node::SystemDirective { .. } => "system",
        Node::Message { role, .. } => match role {
            Role::User => "you",
            Role::Assistant => "assistant",
            Role::System => "system",
        },
    }
}

fn role_color(node: &Node) -> Color {
    match node {
        Node::SystemDirective { .. } => Color::DarkGray,
        Node::Message { role, .. } => match role {
            Role::User => Color::Cyan,
            Role::Assistant => Color::Green,
            Role::System => Color::DarkGray,
        },
    }
}

fn metadata_string(node: &Node) -> String {
    match (node.input_tokens(), node.output_tokens()) {
        (Some(inp), Some(out)) => format!("{inp}in / {out}out"),
        (Some(inp), None) => format!("{inp}in"),
        (None, Some(out)) => format!("{out}out"),
        (None, None) => String::new(),
    }
}

fn top_border(label: &str, metadata: &str, width: usize, color: Color) -> Line<'static> {
    // ┌─ label ──...── metadata ┐
    // minimum: ┌─ label ─┐ = 5 + label.len()
    let mut spans = Vec::new();
    let border_style = Style::default().fg(color);

    let label_part = format!("┌─ {label} ");
    let right_cap = "┐";

    let used = label_part.len() + metadata.len() + right_cap.len();
    let fill = if metadata.is_empty() {
        // ┌─ label ─...─┐
        let used_no_meta = label_part.len() + right_cap.len();
        width.saturating_sub(used_no_meta)
    } else {
        // need a space before metadata
        width.saturating_sub(used + 1)
    };

    spans.push(Span::styled(label_part, border_style));
    spans.push(Span::styled("─".repeat(fill), border_style));

    if !metadata.is_empty() {
        spans.push(Span::styled(
            format!(" {metadata}"),
            Style::default().fg(Color::DarkGray),
        ));
    }

    spans.push(Span::styled(right_cap.to_string(), border_style));

    Line::from(spans)
}

fn bottom_border(width: usize, color: Color) -> Line<'static> {
    let border_style = Style::default().fg(color);
    let fill = width.saturating_sub(2); // └ + ┘
    Line::from(vec![
        Span::styled("└", border_style),
        Span::styled("─".repeat(fill), border_style),
        Span::styled("┘", border_style),
    ])
}

fn content_line(text: &str, content_width: usize, color: Color) -> Line<'static> {
    let border_style = Style::default().fg(color);
    let display_len = text.chars().count();
    let padding = content_width.saturating_sub(display_len);
    Line::from(vec![
        Span::styled("│ ", border_style),
        Span::raw(text.to_string()),
        Span::raw(" ".repeat(padding)),
        Span::styled(" │", border_style),
    ])
}

fn wrap_text(text: &str, max_width: usize) -> Vec<String> {
    let mut result = Vec::new();
    for line in text.lines() {
        if line.is_empty() {
            result.push(String::new());
            continue;
        }
        let chars: Vec<char> = line.chars().collect();
        if chars.len() <= max_width {
            result.push(line.to_string());
        } else {
            let mut start = 0;
            while start < chars.len() {
                let end = (start + max_width).min(chars.len());
                result.push(chars[start..end].iter().collect());
                start = end;
            }
        }
    }
    if result.is_empty() {
        result.push(String::new());
    }
    result
}

fn render_message_box(
    node: &Node,
    content_width: usize,
    box_width: usize,
    lines: &mut Vec<Line<'static>>,
) {
    let label = role_label(node);
    let color = role_color(node);
    let metadata = metadata_string(node);

    lines.push(top_border(label, &metadata, box_width, color));

    let wrapped = wrap_text(node.content(), content_width);
    for text in &wrapped {
        lines.push(content_line(text, content_width, color));
    }

    lines.push(bottom_border(box_width, color));
}

fn render_streaming_box(
    streaming: &str,
    content_width: usize,
    box_width: usize,
    lines: &mut Vec<Line<'static>>,
) {
    let color = Color::Green;
    lines.push(top_border("assistant", "", box_width, color));

    let wrapped = wrap_text(streaming, content_width);
    for text in &wrapped {
        lines.push(content_line(text, content_width, color));
    }

    // Cursor line
    let border_style = Style::default().fg(color);
    let padding = content_width.saturating_sub(1);
    lines.push(Line::from(vec![
        Span::styled("│ ", border_style),
        Span::styled("▌", Style::default().fg(Color::Green)),
        Span::raw(" ".repeat(padding)),
        Span::styled(" │", border_style),
    ]));

    lines.push(bottom_border(box_width, color));
}
