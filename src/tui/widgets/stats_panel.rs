//! Stats panel widget: token usage, message/tool counts, service status.
//!
//! Used by the Agents tab to show system-wide statistics.

use crate::graph::{BackgroundTaskKind, ConversationGraph, Node, TaskStatus};
use crate::tui::ui::format_token_count;
use crate::tui::TuiState;

use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Paragraph};

/// Render stats panel with token usage, message count, and service status.
pub fn render(frame: &mut Frame, area: Rect, graph: &ConversationGraph, tui_state: &TuiState) {
    let block = Block::default().title("Stats").borders(Borders::ALL);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.height == 0 || inner.width < 8 {
        return;
    }

    let input_tok = tui_state.token_usage.input.current;
    let output_tok = tui_state.token_usage.output.current;

    let msg_count = graph.nodes_by(|n| matches!(n, Node::Message { .. })).len();
    let tool_count = graph.nodes_by(|n| matches!(n, Node::ToolCall { .. })).len();

    let dim = Style::default().fg(Color::DarkGray);

    let mut lines = vec![
        Line::from(vec![
            Span::styled("Tokens: ", dim),
            Span::styled(
                format!(
                    "{}in / {}out",
                    format_token_count(input_tok),
                    format_token_count(output_tok)
                ),
                Style::default().fg(Color::Cyan),
            ),
        ]),
        Line::from(vec![
            Span::styled("Messages: ", dim),
            Span::styled(msg_count.to_string(), Style::default().fg(Color::Green)),
        ]),
        Line::from(vec![
            Span::styled("Tools: ", dim),
            Span::styled(tool_count.to_string(), Style::default().fg(Color::Magenta)),
        ]),
    ];

    // Service status.
    if inner.height as usize > lines.len() + 1 {
        lines.push(Line::raw(""));
        let services = [
            ("Git index", BackgroundTaskKind::GitIndex),
            ("Tool disc.", BackgroundTaskKind::ToolDiscovery),
        ];
        for (label, kind) in services {
            let status = graph
                .nodes_by(|n| matches!(n, Node::BackgroundTask { kind: k, .. } if *k == kind))
                .last()
                .and_then(|n| match n {
                    Node::BackgroundTask { status, .. } => Some(*status),
                    _ => None,
                });
            let (icon, color) = match status {
                Some(TaskStatus::Completed) => ("✓", Color::Green),
                Some(TaskStatus::Running) => ("⟳", Color::Cyan),
                Some(TaskStatus::Failed) => ("✗", Color::Red),
                _ => ("○", Color::DarkGray),
            };
            lines.push(Line::from(vec![
                Span::styled(format!("{icon} "), Style::default().fg(color)),
                Span::styled(label, Style::default().fg(Color::White)),
            ]));
        }
    }

    frame.render_widget(Paragraph::new(Text::from(lines)), inner);
}
