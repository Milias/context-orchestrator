use crate::graph::tool_types::ToolCallStatus;
use crate::graph::{ConversationGraph, Node, TaskStatus};
use crate::tui::TuiState;

use chrono::{DateTime, Utc};
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};
use std::time::Duration;

pub fn render(frame: &mut Frame, area: Rect, graph: &ConversationGraph, tui_state: &TuiState) {
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
                // Services are never shown in history — they're noise
            }
            Some(entry) if entry.is_active => active.push(entry),
            Some(entry) => history.push(entry),
            None => {}
        }
    }

    active.sort_by_key(|e| e.created_at);
    history.sort_by(|a, b| b.created_at.cmp(&a.created_at));

    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(area);

    render_active_panel(frame, columns[0], &services, &active);
    render_history_panel(frame, columns[1], &history, tui_state.context_list_offset);
}

// ── Left panel: services row + active tasks ──────────────────────────

fn render_active_panel(
    frame: &mut Frame,
    area: Rect,
    services: &[ServiceEntry],
    tasks: &[TaskEntry],
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

    for entry in tasks {
        items.push(ListItem::new(format_entry_line(entry, width)));
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

enum TaskDuration {
    Pending,
    Elapsed(Duration),
    Finished(Duration),
}

impl TaskEntry {
    fn from_node(node: &Node, now: DateTime<Utc>) -> Option<Self> {
        match node {
            Node::ToolCall {
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
                    name: arguments.display_summary(),
                    is_active,
                    is_service: false,
                    created_at: *created_at,
                    duration,
                    kind: TaskKind::ToolCall(status.clone()),
                })
            }
            Node::BackgroundTask {
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
            TaskKind::ToolCall(s) => match s {
                ToolCallStatus::Pending => ("○", Color::DarkGray),
                ToolCallStatus::Running => ("◉", Color::Yellow),
                ToolCallStatus::Completed => ("✓", Color::Green),
                ToolCallStatus::Failed => ("✗", Color::Red),
                ToolCallStatus::Cancelled => ("⊘", Color::DarkGray),
            },
            TaskKind::BackgroundTask(s) => match s {
                TaskStatus::Pending => ("○", Color::DarkGray),
                TaskStatus::Running => ("◉", Color::Yellow),
                TaskStatus::Completed => ("✓", Color::Green),
                TaskStatus::Failed => ("✗", Color::Red),
                TaskStatus::Stopped => ("■", Color::DarkGray),
            },
        }
    }
}

fn elapsed(now: DateTime<Utc>, start: DateTime<Utc>) -> TaskDuration {
    let d = (now - start).to_std().unwrap_or_default();
    TaskDuration::Elapsed(d)
}

fn finished(end: DateTime<Utc>, start: DateTime<Utc>) -> TaskDuration {
    let d = (end - start).to_std().unwrap_or_default();
    TaskDuration::Finished(d)
}

// ── Formatting helpers ───────────────────────────────────────────────

fn format_duration(d: &TaskDuration) -> String {
    match d {
        TaskDuration::Pending => "···".to_string(),
        TaskDuration::Elapsed(d) | TaskDuration::Finished(d) => {
            let total_ms = d.as_millis();
            if total_ms < 1000 {
                format!("{total_ms}ms")
            } else {
                let total_secs = d.as_secs();
                if total_secs < 10 {
                    let secs = d.as_secs_f64();
                    format!("{secs:.1}s")
                } else if total_secs < 60 {
                    format!("{total_secs}s")
                } else {
                    let m = total_secs / 60;
                    let s = total_secs % 60;
                    format!("{m}m {s:02}s")
                }
            }
        }
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut t: String = s.chars().take(max.saturating_sub(1)).collect();
        t.push('…');
        t
    }
}

fn visible_width(s: &str) -> usize {
    s.chars().count()
}

fn is_task_node(node: &Node) -> bool {
    matches!(node, Node::ToolCall { .. } | Node::BackgroundTask { .. })
}

#[cfg(test)]
#[path = "task_list_tests.rs"]
mod tests;
