use crate::graph::{ConversationGraph, Node, Role};
use crate::tui::TuiState;
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};

pub fn render(frame: &mut Frame, area: Rect, graph: &ConversationGraph, tui_state: &TuiState) {
    let history = graph
        .get_branch_history(graph.active_branch())
        .unwrap_or_default();

    let mut lines: Vec<Line> = Vec::new();

    for node in &history {
        let (prefix, style) = match node {
            Node::SystemDirective { .. } => (
                "[system]",
                Style::default()
                    .fg(Color::DarkGray)
                    .add_modifier(Modifier::ITALIC),
            ),
            Node::Message { role, .. } => match role {
                Role::User => ("[you]", Style::default().fg(Color::Cyan)),
                Role::Assistant => ("[assistant]", Style::default().fg(Color::Green)),
                Role::System => ("[system]", Style::default().fg(Color::DarkGray)),
            },
        };

        lines.push(Line::from(""));

        // Render content line by line
        let content_lines: Vec<&str> = node.content().lines().collect();
        for (i, content_line) in content_lines.iter().enumerate() {
            let text = if i == 0 {
                format!("{} {}", prefix, content_line)
            } else {
                format!("{} {}", " ".repeat(prefix.len()), content_line)
            };
            lines.push(Line::styled(text, style));
        }
        if content_lines.is_empty() {
            lines.push(Line::styled(format!("{} ", prefix), style));
        }
    }

    // Append streaming response if present
    if let Some(ref streaming) = tui_state.streaming_response {
        lines.push(Line::from(""));
        let streaming_lines: Vec<&str> = streaming.lines().collect();
        for (i, line) in streaming_lines.iter().enumerate() {
            let text = if i == 0 {
                format!("[assistant] {}", line)
            } else {
                format!("            {}", line)
            };
            lines.push(Line::styled(text, Style::default().fg(Color::Green)));
        }
        if streaming_lines.is_empty() {
            lines.push(Line::styled(
                "[assistant] ",
                Style::default().fg(Color::Green),
            ));
        }
        lines.push(Line::styled("▌", Style::default().fg(Color::Green)));
    }

    let paragraph = Paragraph::new(lines)
        .block(Block::default().title("Conversation").borders(Borders::ALL))
        .wrap(Wrap { trim: false })
        .scroll((tui_state.scroll_offset, 0));

    frame.render_widget(paragraph, area);
}
