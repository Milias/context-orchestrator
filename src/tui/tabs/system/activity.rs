//! Activity stream section for the System tab.
//!
//! Chronological event stream showing all graph events sorted newest first.
//! Collects tool calls, messages, background tasks, questions, and answers
//! into a unified timeline with scrolling support.

use crate::graph::tool_types::ToolCallStatus;
use crate::graph::{ConversationGraph, Node, Role, TaskStatus};
use crate::tui::widgets::tool_status::{
    elapsed, finished, format_duration, tool_call_status_icon, truncate,
};
use crate::tui::TuiState;

use chrono::{DateTime, Utc};
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Paragraph};
use uuid::Uuid;

/// A single row in the activity event stream.
struct EventRow {
    /// Node UUID that produced this event (for search filtering).
    node_id: Uuid,
    /// Timestamp used for sorting (newest first).
    timestamp: DateTime<Utc>,
    /// Status icon character.
    icon: &'static str,
    /// Color for the icon.
    icon_color: Color,
    /// Primary label (tool name, role, task kind).
    name: String,
    /// Color for the name span (varies by event type).
    name_color: Color,
    /// Secondary label (arguments, message preview, description).
    args: String,
    /// Color for the args span.
    args_color: Color,
    /// Formatted duration string (empty for events without duration).
    duration: String,
}

/// Render the activity event stream: all graph events sorted newest first.
///
/// Collects tool calls, messages, background tasks, questions, and answers
/// into a unified timeline. Applies animated scroll via `tui_state.overview_scroll`.
/// When a search filter is active with a non-empty query, only events whose
/// source node ID is in `search.matching_ids` are shown.
pub fn render_activity(
    frame: &mut Frame,
    area: Rect,
    graph: &ConversationGraph,
    tui_state: &mut TuiState,
) {
    // Build title: show filter indicator when search is active.
    let search_active = tui_state
        .search
        .as_ref()
        .is_some_and(|s| !s.parsed.is_empty());
    let title = if search_active {
        let n = tui_state
            .search
            .as_ref()
            .map_or(0, |s| s.matching_ids.len());
        format!("Activity  FILTER ACTIVE ({n} matches)")
    } else {
        "Activity".to_string()
    };

    let block = Block::default().title(title).borders(Borders::ALL);
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
    collect_questions(graph, &mut events);
    collect_answers(graph, &mut events);

    // Apply search filter: retain only events whose node is in the match set.
    if search_active {
        if let Some(search) = &tui_state.search {
            events.retain(|e| search.matching_ids.contains(&e.node_id));
        }
    }

    events.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));

    // Publish max offset and clamp animated scroll.
    // Cast safety: bounded by event count, well within u16.
    #[allow(clippy::cast_possible_truncation)] // Justified: max_offset <= events.len().
    let max_offset = events.len().saturating_sub(max_rows) as u16;
    tui_state.overview_max = max_offset;
    tui_state.overview_scroll.apply_max(max_offset);

    if events.is_empty() {
        let label = if search_active {
            "(no matching events)"
        } else {
            "(no activity)"
        };
        let empty = Paragraph::new(Span::styled(label, Style::default().fg(Color::DarkGray)));
        frame.render_widget(empty, inner);
        return;
    }

    // Apply scroll: skip to the animated position, take `max_rows`.
    let offset = tui_state.overview_scroll.position() as usize;
    let visible = events.iter().skip(offset).take(max_rows);
    let lines: Vec<Line<'_>> = visible.map(|e| render_event_row(e, width)).collect();

    frame.render_widget(Paragraph::new(Text::from(lines)), inner);
}

// ── Event collectors ────────────────────────────────────────────────

/// Collect tool call events from the graph.
fn collect_tool_calls(graph: &ConversationGraph, now: DateTime<Utc>, events: &mut Vec<EventRow>) {
    for node in graph.nodes_by(|n| matches!(n, Node::ToolCall { .. })) {
        if let Node::ToolCall {
            id,
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
            let (tool_name, tool_args) = arguments.display_parts();
            events.push(EventRow {
                node_id: *id,
                timestamp: *created_at,
                icon,
                icon_color,
                name: tool_name.to_string(),
                name_color: Color::Magenta,
                args: tool_args,
                args_color: Color::White,
                duration,
            });
        }
    }
}

/// Collect message events from the graph.
fn collect_messages(graph: &ConversationGraph, events: &mut Vec<EventRow>) {
    for node in graph.nodes_by(|n| matches!(n, Node::Message { .. })) {
        if let Node::Message {
            id,
            role,
            content,
            created_at,
            ..
        } = node
        {
            let (icon, icon_color, name_color) = match role {
                Role::User => ("U", Color::Cyan, Color::Cyan),
                Role::Assistant => ("A", Color::Green, Color::Green),
                Role::System => ("S", Color::DarkGray, Color::DarkGray),
            };
            let preview = content.lines().next().unwrap_or("(empty)").to_string();
            events.push(EventRow {
                node_id: *id,
                timestamp: *created_at,
                icon,
                icon_color,
                name: format!("[{role:?}]"),
                name_color,
                args: preview,
                args_color: Color::White,
                duration: String::new(),
            });
        }
    }
}

/// Collect background task events from the graph.
fn collect_background_tasks(
    graph: &ConversationGraph,
    now: DateTime<Utc>,
    events: &mut Vec<EventRow>,
) {
    for node in graph.nodes_by(|n| matches!(n, Node::BackgroundTask { .. })) {
        if let Node::BackgroundTask {
            id,
            status,
            description,
            created_at,
            updated_at,
            kind,
            ..
        } = node
        {
            // Running daemons get a steady icon + "active" label instead of
            // an elapsed timer (the timer would count forever).
            let is_running_daemon = kind.is_daemon() && *status == TaskStatus::Running;
            let (icon, icon_color) = if is_running_daemon {
                ("\u{25cf}", Color::Blue) // ●
            } else {
                match status {
                    TaskStatus::Running => ("\u{27f3}", Color::Cyan),
                    TaskStatus::Completed => ("\u{2713}", Color::Green),
                    TaskStatus::Failed => ("\u{2717}", Color::Red),
                    _ => ("\u{25cb}", Color::DarkGray),
                }
            };
            let duration = if is_running_daemon {
                "active".to_string()
            } else {
                match status {
                    TaskStatus::Running => format_duration(&elapsed(now, *created_at)),
                    _ => format_duration(&finished(*updated_at, *created_at)),
                }
            };
            events.push(EventRow {
                node_id: *id,
                timestamp: *created_at,
                icon,
                icon_color,
                name: description.clone(),
                args: String::new(),
                args_color: Color::DarkGray,
                name_color: Color::Blue,
                duration,
            });
        }
    }
}

/// Collect question events from the graph.
fn collect_questions(graph: &ConversationGraph, events: &mut Vec<EventRow>) {
    for node in graph.nodes_by(|n| matches!(n, Node::Question { .. })) {
        if let Node::Question {
            id,
            content,
            status,
            created_at,
            ..
        } = node
        {
            let (icon, icon_color) = match status {
                crate::graph::node::QuestionStatus::Pending => ("?", Color::Yellow),
                crate::graph::node::QuestionStatus::Claimed => ("?", Color::Cyan),
                crate::graph::node::QuestionStatus::Answered => ("\u{2713}", Color::Green),
                _ => ("?", Color::DarkGray),
            };
            let preview = content.lines().next().unwrap_or("(empty)").to_string();
            events.push(EventRow {
                node_id: *id,
                timestamp: *created_at,
                icon,
                icon_color,
                name: "[Question]".to_string(),
                name_color: Color::Yellow,
                args: preview,
                args_color: Color::White,
                duration: String::new(),
            });
        }
    }
}

/// Collect answer events from the graph.
fn collect_answers(graph: &ConversationGraph, events: &mut Vec<EventRow>) {
    for node in graph.nodes_by(|n| matches!(n, Node::Answer { .. })) {
        if let Node::Answer {
            id,
            content,
            created_at,
            ..
        } = node
        {
            let preview = content.lines().next().unwrap_or("(empty)").to_string();
            events.push(EventRow {
                node_id: *id,
                timestamp: *created_at,
                icon: "\u{2713}",
                icon_color: Color::Green,
                name: "[Answer]".to_string(),
                name_color: Color::Green,
                args: preview,
                args_color: Color::White,
                duration: String::new(),
            });
        }
    }
}

// ── Row rendering ───────────────────────────────────────────────────

/// Render a single event row as a ratatui `Line`.
///
/// Format: `HH:MM:SS ICON name args   duration`
/// Name and args are separate spans with distinct colors.
fn render_event_row(event: &EventRow, width: usize) -> Line<'static> {
    let time = event.timestamp.format("%H:%M:%S").to_string();
    let dur = &event.duration;

    // Fixed-width: "HH:MM:SS ICON " + duration.
    let fixed = time.len() + 1 + 2 + dur.len() + 1;
    let content_budget = width.saturating_sub(fixed);

    let name_style = if event.name_color == Color::Magenta {
        Style::default().fg(Color::Magenta).bold()
    } else {
        Style::default().fg(event.name_color)
    };

    // Split budget: name gets its natural width, args gets the rest.
    let name = &event.name;
    let sep = if event.args.is_empty() { "" } else { " " };
    let args_budget = content_budget.saturating_sub(name.chars().count() + sep.len());
    let args = truncate(&event.args, args_budget);
    let padding =
        content_budget.saturating_sub(name.chars().count() + sep.len() + args.chars().count());

    let dim = Style::default().fg(Color::DarkGray);

    let mut spans = vec![
        Span::styled(time, dim),
        Span::raw(" "),
        Span::styled(
            format!("{} ", event.icon),
            Style::default().fg(event.icon_color),
        ),
        Span::styled(name.clone(), name_style),
    ];
    if !event.args.is_empty() {
        spans.push(Span::styled(sep, dim));
        spans.push(Span::styled(args, Style::default().fg(event.args_color)));
    }
    spans.push(Span::raw(" ".repeat(padding)));
    if !dur.is_empty() {
        let dur_color = if event.icon_color == Color::Yellow {
            Color::Yellow
        } else {
            Color::DarkGray
        };
        spans.push(Span::styled(dur.clone(), Style::default().fg(dur_color)));
    }
    Line::from(spans)
}
