use crate::graph::ConversationGraph;
use crate::tui::widgets::{context_panel, conversation, input_box};
use crate::tui::TuiState;
use ratatui::prelude::*;
use ratatui::widgets::Paragraph;

pub fn draw(frame: &mut Frame, graph: &ConversationGraph, tui_state: &mut TuiState) {
    let area = frame.area();
    let show_context = tui_state.context_panel_visible && area.height >= 20;

    let vertical = if show_context {
        Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),
                Constraint::Percentage(30),
                Constraint::Min(5),
                Constraint::Length(5),
            ])
            .split(area)
    } else {
        Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),
                Constraint::Min(5),
                Constraint::Length(5),
            ])
            .split(area)
    };

    if show_context {
        draw_status_bar(frame, vertical[0], graph, tui_state);
        context_panel::render(frame, vertical[1], graph, tui_state);
        conversation::render(frame, vertical[2], graph, tui_state);
        input_box::render(frame, vertical[3], area, tui_state);
    } else {
        draw_status_bar(frame, vertical[0], graph, tui_state);
        conversation::render(frame, vertical[1], graph, tui_state);
        input_box::render(frame, vertical[2], area, tui_state);
    }
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
