use crate::graph::tool_types::ToolCallStatus;
use crate::graph::{ConversationGraph, Node, TaskStatus};
use crate::tui::widgets::tool_status::{
    bg_task_status_icon, elapsed, finished, format_duration, tool_call_status_icon, truncate,
    visible_width, TaskDuration,
};
use crate::tui::TuiState;

use chrono::{DateTime, Utc};
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};
use uuid::Uuid;

pub fn render(frame: &mut Frame, area: Rect, graph: &ConversationGraph, tui_state: &mut TuiState) {
    let now = Utc::now();
    let mut services: Vec<ServiceEntry> = Vec::new();
    let mut active: Vec<TaskEntry> = Vec::new();
    let mut history: Vec<TaskEntry> = Vec::new();

    for node in graph.nodes_by(is_task_node) {
        match TaskEntry::from_node(node, now) {
            Some(entry) if entry.is_service => {
                if entry.is_active {
                    services.push(ServiceEntry::from(&entry));
                }
            }
            Some(entry) if entry.is_active => active.push(entry),
            Some(entry) => history.push(entry),
            None => {}
        }
    }

    active.sort_by_key(|e| e.created_at);
    history.sort_by(|a, b| b.created_at.cmp(&a.created_at));

    // Update active task IDs for input handler (maps selection index → UUID)
    tui_state.active_task_ids = active.iter().map(|e| e.node_id).collect();
    // Clamp selection to valid range
    if let Some(sel) = tui_state.task_selection {
        if sel >= tui_state.active_task_ids.len() {
            tui_state.task_selection = if tui_state.active_task_ids.is_empty() {
                None
            } else {
                Some(tui_state.active_task_ids.len() - 1)
            };
        }
    }

    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(area);

    render_active_panel(
        frame,
        columns[0],
        &services,
        &active,
        tui_state.task_selection,
    );
    render_history_panel(frame, columns[1], &history, tui_state.context_list_offset);
}

// ── Left panel: services row + active tasks ──────────────────────────

fn render_active_panel(
    frame: &mut Frame,
    area: Rect,
    services: &[ServiceEntry],
    tasks: &[TaskEntry],
    selection: Option<usize>,
) {
    let block = Block::default()
        .title("Running")
        .borders(Borders::RIGHT)
        .border_style(Style::default().fg(Color::DarkGray));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.width < 4 || inner.height == 0 {
        return;
    }

    if services.is_empty() && tasks.is_empty() {
        frame.render_widget(
            Paragraph::new(Span::styled("(idle)", Style::default().fg(Color::DarkGray))),
            inner,
        );
        return;
    }

    let width = inner.width as usize;
    let mut items: Vec<ListItem> = Vec::new();

    if !services.is_empty() {
        items.push(ListItem::new(format_services_line(services, width)));
    }

    for (i, entry) in tasks.iter().enumerate() {
        let mut item = ListItem::new(format_entry_line(entry, width));
        if selection == Some(i) {
            item = item.style(Style::default().bg(Color::Rgb(40, 40, 60)));
        }
        items.push(item);
    }

    frame.render_widget(List::new(items), inner);
}

fn format_services_line(services: &[ServiceEntry], width: usize) -> Line<'static> {
    let mut spans: Vec<Span<'static>> = Vec::new();
    let style = Style::default().fg(Color::DarkGray);
    for (i, svc) in services.iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled("  ", style));
        }
        spans.push(Span::styled("⟳ ", Style::default().fg(Color::Cyan)));
        let label = truncate(&svc.label, width / services.len().max(1));
        spans.push(Span::styled(label, style));
    }
    Line::from(spans)
}

// ── Right panel: recent completed tasks ──────────────────────────────

fn render_history_panel(frame: &mut Frame, area: Rect, entries: &[TaskEntry], offset: usize) {
    let block = Block::default()
        .title("Recent")
        .border_style(Style::default().fg(Color::DarkGray));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.width < 4 || inner.height == 0 {
        return;
    }

    if entries.is_empty() {
        frame.render_widget(
            Paragraph::new(Span::styled("(none)", Style::default().fg(Color::DarkGray))),
            inner,
        );
        return;
    }

    let width = inner.width as usize;
    let items: Vec<ListItem> = entries
        .iter()
        .skip(offset)
        .map(|e| ListItem::new(format_entry_line(e, width)))
        .collect();
    frame.render_widget(List::new(items), inner);
}

// ── Entry formatting ─────────────────────────────────────────────────

fn format_entry_line(entry: &TaskEntry, width: usize) -> Line<'static> {
    let (icon, icon_color) = entry.status_icon();
    let dur = format_duration(&entry.duration);
    let dur_color = if entry.is_active {
        Color::Yellow
    } else {
        Color::DarkGray
    };

    // icon(2) + name + gap(1) + duration
    let dur_width = dur.len();
    let name_budget = width.saturating_sub(2 + 1 + dur_width);
    let name = truncate(&entry.name, name_budget);
    let padding = name_budget.saturating_sub(visible_width(&name));

    Line::from(vec![
        Span::styled(format!("{icon} "), Style::default().fg(icon_color)),
        Span::styled(name, Style::default().fg(Color::Magenta).bold()),
        Span::raw(" ".repeat(padding)),
        Span::styled(dur, Style::default().fg(dur_color)),
    ])
}

// ── Data types ───────────────────────────────────────────────────────

struct ServiceEntry {
    label: String,
}

impl ServiceEntry {
    fn from(entry: &TaskEntry) -> Self {
        Self {
            label: entry.name.clone(),
        }
    }
}

struct TaskEntry {
    node_id: Uuid,
    name: String,
    is_active: bool,
    is_service: bool,
    created_at: DateTime<Utc>,
    duration: TaskDuration,
    kind: TaskKind,
}

enum TaskKind {
    ToolCall(ToolCallStatus),
    BackgroundTask(TaskStatus),
}

impl TaskEntry {
    fn from_node(node: &Node, now: DateTime<Utc>) -> Option<Self> {
        match node {
            Node::ToolCall {
                id,
                arguments,
                status,
                created_at,
                completed_at,
                ..
            } => {
                let is_active = matches!(status, ToolCallStatus::Pending | ToolCallStatus::Running);
                let duration = match (is_active, completed_at) {
                    (true, _) if *status == ToolCallStatus::Pending => TaskDuration::Pending,
                    (true, _) => elapsed(now, *created_at),
                    (false, Some(end)) => finished(*end, *created_at),
                    (false, None) => finished(now, *created_at),
                };
                Some(Self {
                    node_id: *id,
                    name: arguments.display_summary(),
                    is_active,
                    is_service: false,
                    created_at: *created_at,
                    duration,
                    kind: TaskKind::ToolCall(status.clone()),
                })
            }
            Node::BackgroundTask {
                id,
                kind,
                status,
                description,
                created_at,
                updated_at,
                ..
            } => {
                let is_active = matches!(status, TaskStatus::Pending | TaskStatus::Running);
                let duration = if *status == TaskStatus::Pending {
                    TaskDuration::Pending
                } else if is_active {
                    elapsed(now, *created_at)
                } else {
                    finished(*updated_at, *created_at)
                };
                Some(Self {
                    node_id: *id,
                    name: description.clone(),
                    is_active,
                    is_service: kind.is_service(),
                    created_at: *created_at,
                    duration,
                    kind: TaskKind::BackgroundTask(*status),
                })
            }
            _ => None,
        }
    }

    fn status_icon(&self) -> (&'static str, Color) {
        match &self.kind {
            TaskKind::ToolCall(s) => tool_call_status_icon(s),
            TaskKind::BackgroundTask(s) => bg_task_status_icon(*s),
        }
    }
}

fn is_task_node(node: &Node) -> bool {
    matches!(node, Node::ToolCall { .. } | Node::BackgroundTask { .. })
}

#[cfg(test)]
#[path = "task_list_tests.rs"]
mod tests;
