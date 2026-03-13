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
    let left_text = tui_state
        .status_message
        .as_deref()
        .unwrap_or("")
        .to_string();
    let left_text = if left_text.is_empty() {
        format!("Context Manager v0.1  [branch: {}]", graph.active_branch())
    } else {
        left_text
    };

    let bg = Style::default().bg(Color::Rgb(30, 30, 80));
    let mut spans = vec![Span::styled(left_text, bg.fg(Color::White))];

    if let Some(ref err) = tui_state.error_message {
        let right = format!("  {err}");
        spans.push(Span::styled(right, bg.fg(Color::Red)));
    }

    let line = Line::from(spans);
    let status = Paragraph::new(line).style(bg);
    frame.render_widget(status, area);
}
