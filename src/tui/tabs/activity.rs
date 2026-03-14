//! Activity tab: real-time event stream.
//!
//! Displays all graph events (tool calls, messages, background tasks)
//! as a time-sorted table. Shows the most recent events at the top.

use crate::graph::tool_types::ToolCallStatus;
use crate::graph::{ConversationGraph, Node, Role};
use crate::tui::widgets::tool_status::{
    elapsed, finished, format_duration, tool_call_status_icon, truncate,
};
use crate::tui::TuiState;

use chrono::Utc;
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Paragraph};

/// Render the Activity tab content into the given area.
pub fn render(frame: &mut Frame, area: Rect, graph: &ConversationGraph, _tui_state: &TuiState) {
    let block = Block::default().title("Activity").borders(Borders::ALL);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.height == 0 || inner.width < 20 {
        return;
    }

    let now = Utc::now();
    let width = inner.width as usize;
    let max_rows = inner.height as usize;

    // Collect all displayable events, sorted newest first.
    let mut events: Vec<EventRow> = Vec::new();
    collect_tool_calls(graph, now, &mut events);
    collect_messages(graph, &mut events);
    collect_background_tasks(graph, now, &mut events);

    events.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
    events.truncate(max_rows);

    if events.is_empty() {
        let empty = Paragraph::new(Span::styled(
            "(no activity)",
            Style::default().fg(Color::DarkGray),
        ));
        frame.render_widget(empty, inner);
        return;
    }

    let lines: Vec<Line<'_>> = events.iter().map(|e| render_event_row(e, width)).collect();

    frame.render_widget(Paragraph::new(Text::from(lines)), inner);
}

/// A single row in the activity stream.
struct EventRow {
    timestamp: chrono::DateTime<Utc>,
    kind: &'static str,
    icon: &'static str,
    icon_color: Color,
    name: String,
    duration: String,
}

/// Collect tool call events from the graph.
fn collect_tool_calls(
    graph: &ConversationGraph,
    now: chrono::DateTime<Utc>,
    events: &mut Vec<EventRow>,
) {
    for node in graph.nodes_by(|n| matches!(n, Node::ToolCall { .. })) {
        if let Node::ToolCall {
            status,
            arguments,
            created_at,
            completed_at,
            ..
        } = node
        {
            let (icon, icon_color) = tool_call_status_icon(status);
            let duration = match (status, completed_at) {
                (ToolCallStatus::Pending, _) => {
                    format_duration(&crate::tui::widgets::tool_status::TaskDuration::Pending)
                }
                (ToolCallStatus::Running, _) => format_duration(&elapsed(now, *created_at)),
                (_, Some(end)) => format_duration(&finished(*end, *created_at)),
                (_, None) => format_duration(&finished(now, *created_at)),
            };
            events.push(EventRow {
                timestamp: *created_at,
                kind: "ToolCall",
                icon,
                icon_color,
                name: arguments.display_summary(),
                duration,
            });
        }
    }
}

/// Collect message events from the graph.
fn collect_messages(graph: &ConversationGraph, events: &mut Vec<EventRow>) {
    for node in graph.nodes_by(|n| matches!(n, Node::Message { .. })) {
        if let Node::Message {
            role,
            content,
            created_at,
            ..
        } = node
        {
            let (icon, icon_color) = match role {
                Role::User => ("U", Color::Cyan),
                Role::Assistant => ("A", Color::Green),
                Role::System => ("S", Color::DarkGray),
            };
            let preview = content.lines().next().unwrap_or("(empty)");
            events.push(EventRow {
                timestamp: *created_at,
                kind: "Message",
                icon,
                icon_color,
                name: format!("[{role:?}] {preview}"),
                duration: String::new(),
            });
        }
    }
}

/// Collect background task events from the graph.
fn collect_background_tasks(
    graph: &ConversationGraph,
    now: chrono::DateTime<Utc>,
    events: &mut Vec<EventRow>,
) {
    for node in graph.nodes_by(|n| matches!(n, Node::BackgroundTask { .. })) {
        if let Node::BackgroundTask {
            status,
            description,
            created_at,
            updated_at,
            ..
        } = node
        {
            let (icon, icon_color) = match status {
                crate::graph::TaskStatus::Running => ("⟳", Color::Cyan),
                crate::graph::TaskStatus::Completed => ("✓", Color::Green),
                crate::graph::TaskStatus::Failed => ("✗", Color::Red),
                _ => ("○", Color::DarkGray),
            };
            let duration = match status {
                crate::graph::TaskStatus::Running => format_duration(&elapsed(now, *created_at)),
                _ => format_duration(&finished(*updated_at, *created_at)),
            };
            events.push(EventRow {
                timestamp: *created_at,
                kind: "Task",
                icon,
                icon_color,
                name: description.clone(),
                duration,
            });
        }
    }
}

/// Render a single event row as a ratatui `Line`.
fn render_event_row(event: &EventRow, width: usize) -> Line<'static> {
    let time = event.timestamp.format("%H:%M:%S").to_string();
    let dur = &event.duration;

    // Layout: "HH:MM:SS KIND  ICON NAME          DURATION"
    let fixed = time.len() + 1 + event.kind.len() + 2 + 2 + dur.len() + 1;
    let name_budget = width.saturating_sub(fixed);
    let name = truncate(&event.name, name_budget);
    let padding = name_budget.saturating_sub(name.chars().count());

    let dim = Style::default().fg(Color::DarkGray);

    let mut spans = vec![
        Span::styled(time, dim),
        Span::raw(" "),
        Span::styled(format!("{:<8}", event.kind), dim),
        Span::styled(
            format!("{} ", event.icon),
            Style::default().fg(event.icon_color),
        ),
        Span::styled(name, Style::default().fg(Color::White)),
        Span::raw(" ".repeat(padding)),
    ];
    if !dur.is_empty() {
        spans.push(Span::styled(dur.clone(), dim));
    }
    Line::from(spans)
}
