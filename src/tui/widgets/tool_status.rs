use crate::graph::tool_types::ToolCallStatus;
use crate::graph::TaskStatus;

use chrono::{DateTime, Utc};
use ratatui::prelude::*;
use std::time::Duration;

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
