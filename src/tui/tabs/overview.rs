//! Overview tab: unified dashboard combining agents, work, and stats.
//!
//! Stacks vertically: agent card, running tasks, work tree (compact),
//! then a horizontal split of the activity event stream and stats at the bottom.

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

/// Maximum number of work tree lines shown in the overview.
const WORK_TREE_MAX_LINES: u16 = 8;

/// Render the Overview tab.
///
/// Layout:
/// ```text
/// Activity | Recent    | Agent card
///          | tool calls| Running tasks
///          |           | Stats
/// Work tree (full width, capped at 8 lines)
/// ```
pub fn render(frame: &mut Frame, area: Rect, graph: &ConversationGraph, tui_state: &mut TuiState) {
    let work_h = work_tree_height(graph);

    // Top: main content (flex). Bottom: work tree (capped).
    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(5), Constraint::Length(work_h)])
        .split(area);

    // Main: 3 columns — activity (40%) | recent (30%) | right panel (30%).
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(40),
            Constraint::Percentage(30),
            Constraint::Percentage(30),
        ])
        .split(outer[0]);

    // Left: activity stream.
    render_activity_stream(frame, cols[0], graph, tui_state);
    tui_state.panel_rects.activity = cols[0];

    // Center: recent tool call completions.
    agents::render_recent_completions(frame, cols[1], graph, tui_state);
    tui_state.panel_rects.recent = cols[1];

    // Right: agent card + running tasks + stats (stacked vertically).
    let right_col = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(agents::agent_card_height(tui_state)),
            Constraint::Length(agents::running_tasks_height(graph)),
            Constraint::Min(3),
        ])
        .split(cols[2]);

    agents::render_agent_card(frame, right_col[0], tui_state);
    agents::render_running_tasks(frame, right_col[1], graph, tui_state);
    crate::tui::widgets::stats_panel::render(frame, right_col[2], graph, tui_state);

    // Bottom: work tree.
    render_work_section(frame, outer[1], graph, tui_state);
    tui_state.panel_rects.work = outer[1];
}

// ── Activity stream ──────────────────────────────────────────────────

/// A single row in the activity event stream.
struct EventRow {
    /// Timestamp used for sorting (newest first).
    timestamp: DateTime<Utc>,
    /// Short category label (e.g. "`ToolCall`", "Message", "Task").
    kind: &'static str,
    /// Status icon character.
    icon: &'static str,
    /// Color for the icon.
    icon_color: Color,
    /// Human-readable name or summary.
    name: String,
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
    now: DateTime<Utc>,
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
                TaskStatus::Running => ("\u{27f3}", Color::Cyan),
                TaskStatus::Completed => ("\u{2713}", Color::Green),
                TaskStatus::Failed => ("\u{2717}", Color::Red),
                _ => ("\u{25cb}", Color::DarkGray),
            };
            let duration = match status {
                TaskStatus::Running => format_duration(&elapsed(now, *created_at)),
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

// ── Work tree section ────────────────────────────────────────────────

/// Compute the work tree section height: border (2) + items, capped at `WORK_TREE_MAX_LINES`.
/// Returns 0 when there are no work items (hides the section entirely).
fn work_tree_height(graph: &ConversationGraph) -> u16 {
    let tree = work::build_work_tree(graph);
    if tree.is_empty() {
        return 0;
    }
    let count = count_tree_items(&tree);
    let n: u16 = u16::try_from(count).unwrap_or(u16::MAX);
    // border (2) + items, clamped to max lines + border.
    n.min(WORK_TREE_MAX_LINES).saturating_add(2)
}

/// Count all items in the tree (roots + all descendants) for height calculation.
fn count_tree_items(items: &[work::WorkTreeItem]) -> usize {
    items
        .iter()
        .map(|item| 1 + count_tree_items(item.children()))
        .sum()
}

/// Render a compact work tree section inside a bordered "Work" block.
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
