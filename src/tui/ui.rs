use crate::graph::ConversationGraph;
use crate::tui::widgets::{conversation, input_box};
use crate::tui::TuiState;
use ratatui::prelude::*;
use ratatui::widgets::Paragraph;

pub fn draw(frame: &mut Frame, graph: &ConversationGraph, tui_state: &TuiState) {
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(5),
            Constraint::Length(5),
        ])
        .split(frame.area());

    let status_area = vertical[0];
    let conversation_area = vertical[1];
    let input_area = vertical[2];

    draw_status_bar(frame, status_area, graph, tui_state);
    conversation::render(frame, conversation_area, graph, tui_state);
    input_box::render(frame, input_area, tui_state);
}

fn draw_status_bar(frame: &mut Frame, area: Rect, graph: &ConversationGraph, tui_state: &TuiState) {
    let status_text = if let Some(ref msg) = tui_state.status_message {
        msg.clone()
    } else {
        format!("Context Manager v0.1  [branch: {}]", graph.active_branch())
    };
    let status =
        Paragraph::new(status_text).style(Style::default().bg(Color::Blue).fg(Color::White));
    frame.render_widget(status, area);
}
