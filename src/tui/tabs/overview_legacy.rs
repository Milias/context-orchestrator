//! Overview tab: unified dashboard combining agents, work, and stats.
//!
//! Three-column layout: activity stream (left), work tree (center),
//! and a right panel stacking agents, running tasks, recent completions,
//! available tools, and stats.

use crate::graph::tool_types::ToolCallStatus;
use crate::graph::{ConversationGraph, Node, Role, TaskStatus};
use crate::tui::widgets::tool_status::{
    elapsed, finished, format_duration, tool_call_status_icon, truncate,
};
use crate::tui::TuiState;

use chrono::{DateTime, Utc};
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Paragraph};

use super::{agents, work};

/// Render the Overview tab.
///
/// Layout:
/// ```text
/// Activity (40%) | Work (30%) | Agent card (30%)
///                |            | Running
///                |            | Recent
///                |            | Tools
///                |            | Stats
/// ```
pub fn render(frame: &mut Frame, area: Rect, graph: &ConversationGraph, tui_state: &mut TuiState) {
    use crate::tui::widgets::{stats_panel, tools_panel};

    // 3 columns — activity (40%) | work (30%) | right panel (30%).
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(40),
            Constraint::Percentage(30),
            Constraint::Percentage(30),
        ])
        .split(area);

    // Left: activity stream.
    render_activity_stream(frame, cols[0], graph, tui_state);
    tui_state.panel_rects.activity = cols[0];

    // Center: work tree (fills entire column).
    render_work_section(frame, cols[1], graph, tui_state);
    tui_state.panel_rects.work = cols[1];

    // Right: agents → running → recent → tools → stats (stacked).
    let right_col = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(agents::agent_card_height(tui_state)),
            Constraint::Length(agents::running_tasks_height(graph)),
            Constraint::Min(5),
            Constraint::Length(tools_panel::tools_panel_height()),
            Constraint::Length(9),
        ])
        .split(cols[2]);

    agents::render_agent_card(frame, right_col[0], tui_state);
    agents::render_running_tasks(frame, right_col[1], graph, tui_state);
    agents::render_recent_completions(frame, right_col[2], graph, tui_state);
    tui_state.panel_rects.recent = right_col[2];
    tools_panel::render(frame, right_col[3]);
    stats_panel::render(frame, right_col[4], graph, tui_state);
}

// ── Activity stream ──────────────────────────────────────────────────

/// A single row in the activity event stream.
struct EventRow {
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
fn render_activity_stream(
    frame: &mut Frame,
    area: Rect,
    graph: &ConversationGraph,
    tui_state: &mut TuiState,
) {
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

    // Publish max offset and clamp animated scroll.
    // Cast safety: bounded by event count, well within u16.
    #[allow(clippy::cast_possible_truncation)] // Justified: max_offset ≤ events.len().
    let max_offset = events.len().saturating_sub(max_rows) as u16;
    tui_state.overview_max = max_offset;
    tui_state.overview_scroll.apply_max(max_offset);

    if events.is_empty() {
        let empty = Paragraph::new(Span::styled(
            "(no activity)",
            Style::default().fg(Color::DarkGray),
        ));
        frame.render_widget(empty, inner);
        return;
    }

    // Apply scroll: skip to the animated position, take `max_rows`.
    let offset = tui_state.overview_scroll.position() as usize;
    let visible = events.iter().skip(offset).take(max_rows);
    let lines: Vec<Line<'_>> = visible.map(|e| render_event_row(e, width)).collect();

    frame.render_widget(Paragraph::new(Text::from(lines)), inner);
}

/// Collect tool call events from the graph.
fn collect_tool_calls(graph: &ConversationGraph, now: DateTime<Utc>, events: &mut Vec<EventRow>) {
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
            let (tool_name, tool_args) = arguments.display_parts();
            events.push(EventRow {
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
            status,
            description,
            created_at,
            updated_at,
            kind,
            ..
        } = node
        {
            // Daemons that are still running get a steady icon + "active" label
            // instead of an elapsed timer (the timer would count forever).
            let is_running_daemon = kind.is_daemon() && *status == TaskStatus::Running;
            let (icon, icon_color) = if is_running_daemon {
                ("●", Color::Blue)
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

/// Render a single event row as a ratatui `Line`.
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

// ── Work tree section ────────────────────────────────────────────────

/// Render the work tree inside a bordered "Work" block.
fn render_work_section(
    frame: &mut Frame,
    area: Rect,
    graph: &ConversationGraph,
    tui_state: &mut TuiState,
) {
    if area.height < 3 {
        return;
    }

    let block = Block::default().title("Work").borders(Borders::ALL);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.height == 0 || inner.width < 8 {
        return;
    }

    let tree = work::build_work_tree(graph);

    if tree.is_empty() {
        let empty = Paragraph::new(Span::styled(
            "(no plans or tasks)",
            Style::default().fg(Color::DarkGray),
        ));
        frame.render_widget(empty, inner);
        return;
    }

    let width = inner.width as usize;
    let max_lines = inner.height as usize;
    let mut lines: Vec<Line<'_>> = Vec::new();

    for item in &tree {
        if lines.len() >= max_lines {
            break;
        }
        work::render_item(
            &mut lines,
            item,
            0,
            width,
            max_lines,
            tui_state.work_selected,
        );
    }

    // Publish visible count so input handler can clamp selection.
    tui_state.work_visible_count = lines.len();

    frame.render_widget(Paragraph::new(Text::from(lines)), inner);
}
