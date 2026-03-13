use crate::graph::tool_types::ToolCallStatus;
use crate::graph::{ConversationGraph, EdgeKind, Node, TaskStatus};
use crate::tui::SPINNER_FRAMES;

use chrono::{DateTime, Utc};
use ratatui::prelude::*;
use std::time::Duration;
use uuid::Uuid;

// ── Shared types ────────────────────────────────────────────────────

pub enum TaskDuration {
    Pending,
    Elapsed(Duration),
    Finished(Duration),
}

pub fn elapsed(now: DateTime<Utc>, start: DateTime<Utc>) -> TaskDuration {
    TaskDuration::Elapsed((now - start).to_std().unwrap_or_default())
}

pub fn finished(end: DateTime<Utc>, start: DateTime<Utc>) -> TaskDuration {
    TaskDuration::Finished((end - start).to_std().unwrap_or_default())
}

pub fn tool_call_status_icon(status: &ToolCallStatus) -> (&'static str, Color) {
    match status {
        ToolCallStatus::Pending => ("○", Color::DarkGray),
        ToolCallStatus::Running => ("◉", Color::Yellow),
        ToolCallStatus::Completed => ("✓", Color::Green),
        ToolCallStatus::Failed => ("✗", Color::Red),
        ToolCallStatus::Cancelled => ("⊘", Color::DarkGray),
    }
}

pub fn bg_task_status_icon(status: TaskStatus) -> (&'static str, Color) {
    match status {
        TaskStatus::Pending => ("○", Color::DarkGray),
        TaskStatus::Running => ("◉", Color::Yellow),
        TaskStatus::Completed => ("✓", Color::Green),
        TaskStatus::Failed => ("✗", Color::Red),
        TaskStatus::Stopped => ("■", Color::DarkGray),
    }
}

// ── Formatting helpers ──────────────────────────────────────────────

pub fn format_duration(d: &TaskDuration) -> String {
    match d {
        TaskDuration::Pending => "···".to_string(),
        TaskDuration::Elapsed(d) | TaskDuration::Finished(d) => format_std_duration(d),
    }
}

fn format_std_duration(d: &Duration) -> String {
    let total_ms = d.as_millis();
    if total_ms < 1000 {
        return format!("{total_ms}ms");
    }
    let total_secs = d.as_secs();
    if total_secs < 10 {
        return format!("{:.1}s", d.as_secs_f64());
    }
    if total_secs < 60 {
        return format!("{total_secs}s");
    }
    format!("{}m {:02}s", total_secs / 60, total_secs % 60)
}

pub fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut t: String = s.chars().take(max.saturating_sub(1)).collect();
        t.push('…');
        t
    }
}

pub fn visible_width(s: &str) -> usize {
    s.chars().count()
}

fn format_result_size(chars: usize) -> String {
    if chars < 1000 {
        format!("{chars}")
    } else if chars < 10_000 {
        // Precision loss is acceptable — display-only approximation for result sizes.
        #[allow(clippy::cast_precision_loss)]
        let kb = chars as f64 / 1000.0;
        format!("{kb:.1}K")
    } else {
        format!("{}K", chars / 1000)
    }
}

// ── Conversation tool lines ─────────────────────────────────────────

/// Build styled lines for tool calls during agent execution.
/// Each line shows: icon + tool summary + result size (if done) + duration.
pub fn build_tool_lines(
    graph: &ConversationGraph,
    iteration_node_ids: &[Uuid],
    spinner_tick: usize,
    width: usize,
) -> Vec<Line<'static>> {
    let now = Utc::now();
    let spinner = SPINNER_FRAMES[spinner_tick % SPINNER_FRAMES.len()];
    let mut lines = Vec::new();

    for assistant_id in iteration_node_ids {
        for tc_id in graph.sources_by_edge(*assistant_id, EdgeKind::Invoked) {
            if let Some(Node::ToolCall {
                status,
                arguments,
                created_at,
                completed_at,
                ..
            }) = graph.node(tc_id)
            {
                let is_active = matches!(status, ToolCallStatus::Pending | ToolCallStatus::Running);
                let (icon, icon_color) = if is_active {
                    (spinner, Color::Yellow)
                } else {
                    tool_call_status_icon(status)
                };

                let duration = match (is_active, completed_at) {
                    (true, _) if *status == ToolCallStatus::Pending => TaskDuration::Pending,
                    (true, _) => elapsed(now, *created_at),
                    (false, Some(end)) => finished(*end, *created_at),
                    (false, None) => finished(now, *created_at),
                };

                let result_size = result_size_for(graph, tc_id);
                let name = arguments.display_summary();
                lines.push(format_tool_line(
                    icon,
                    icon_color,
                    &name,
                    result_size.as_deref(),
                    &duration,
                    is_active,
                    width,
                ));
            }
        }
    }
    lines
}

fn result_size_for(graph: &ConversationGraph, tool_call_id: Uuid) -> Option<String> {
    let result_ids = graph.sources_by_edge(tool_call_id, EdgeKind::Produced);
    result_ids.first().and_then(|r_id| {
        if let Some(Node::ToolResult { content, .. }) = graph.node(*r_id) {
            let len = content.char_len();
            if len > 0 {
                return Some(format!("→ {}", format_result_size(len)));
            }
        }
        None
    })
}

fn format_tool_line(
    icon: &str,
    icon_color: Color,
    name: &str,
    result_size: Option<&str>,
    duration: &TaskDuration,
    is_active: bool,
    width: usize,
) -> Line<'static> {
    let dur = format_duration(duration);
    let dur_color = if is_active {
        Color::Yellow
    } else {
        Color::DarkGray
    };

    let size_str = result_size.unwrap_or("");
    let size_width = if size_str.is_empty() {
        0
    } else {
        size_str.len() + 1
    };

    // icon(2) + name + gap(1) + size + gap(1) + duration
    let fixed = 2 + 1 + size_width + dur.len();
    let name_budget = width.saturating_sub(fixed);
    let name = truncate(name, name_budget);
    let padding = name_budget.saturating_sub(visible_width(&name));

    let mut spans = vec![
        Span::styled(format!("{icon} "), Style::default().fg(icon_color)),
        Span::styled(name, Style::default().fg(Color::Magenta).bold()),
        Span::raw(" ".repeat(padding)),
    ];

    if !size_str.is_empty() {
        spans.push(Span::styled(
            format!("{size_str} "),
            Style::default().fg(Color::DarkGray),
        ));
    }

    spans.push(Span::styled(dur, Style::default().fg(dur_color)));
    Line::from(spans)
}
