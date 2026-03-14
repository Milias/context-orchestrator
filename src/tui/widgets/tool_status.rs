use crate::graph::tool_types::ToolCallStatus;
use crate::graph::{ConversationGraph, EdgeKind, Node};
use crate::tui::SPINNER_FRAMES;

use chrono::{DateTime, Utc};
use ratatui::prelude::*;
use std::time::Duration;
use uuid::Uuid;

// ── Shared types ────────────────────────────────────────────────────

/// Lifetime state of a task for display formatting.
/// `Pending` shows a placeholder, `Elapsed`/`Finished` show a duration.
pub enum TaskDuration {
    /// Task has not started yet.
    Pending,
    /// Task is running; duration so far.
    Elapsed(Duration),
    /// Task completed; total duration.
    Finished(Duration),
}

/// Compute an `Elapsed` duration from now back to a start time.
pub fn elapsed(now: DateTime<Utc>, start: DateTime<Utc>) -> TaskDuration {
    TaskDuration::Elapsed((now - start).to_std().unwrap_or_default())
}

/// Compute a `Finished` duration between a start and end time.
pub fn finished(end: DateTime<Utc>, start: DateTime<Utc>) -> TaskDuration {
    TaskDuration::Finished((end - start).to_std().unwrap_or_default())
}

/// Return a status icon and color for a tool call status.
/// Used by the Agents tab, Activity tab, and conversation widget.
pub fn tool_call_status_icon(status: &ToolCallStatus) -> (&'static str, Color) {
    match status {
        ToolCallStatus::Pending => ("○", Color::DarkGray),
        ToolCallStatus::Running => ("◉", Color::Yellow),
        ToolCallStatus::Completed => ("✓", Color::Green),
        ToolCallStatus::Failed => ("✗", Color::Red),
        ToolCallStatus::Cancelled => ("⊘", Color::DarkGray),
    }
}

// ── Formatting helpers ──────────────────────────────────────────────

/// Format a `TaskDuration` for compact display.
/// Returns `"···"` for pending, or a human-readable duration string.
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

/// Truncate a string to `max` characters, appending `…` if shortened.
/// Operates on char boundaries (safe for UTF-8 multibyte strings).
pub fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut t: String = s.chars().take(max.saturating_sub(1)).collect();
        t.push('…');
        t
    }
}

/// Approximate display width of a string in terminal columns.
/// Uses `chars().count()` — correct for ASCII, underestimates CJK/emoji.
pub fn visible_width(s: &str) -> usize {
    s.chars().count()
}

// ── Conversation tool status lines ──────────────────────────────────

const MAX_RESULT_LINES: usize = 10;

/// Build compact status lines for an assistant message's tool calls.
/// When `expanded`, shows result content below each completed tool.
pub fn build_tool_lines(
    graph: &ConversationGraph,
    assistant_id: Uuid,
    spinner_tick: usize,
    width: usize,
    expanded: bool,
) -> Vec<Line<'static>> {
    let now = Utc::now();
    let spinner = SPINNER_FRAMES[spinner_tick % SPINNER_FRAMES.len()];
    let mut lines = Vec::new();

    for tc_id in graph.sources_by_edge(assistant_id, EdgeKind::Invoked) {
        let Some(Node::ToolCall {
            status,
            arguments,
            created_at,
            completed_at,
            ..
        }) = graph.node(tc_id)
        else {
            continue;
        };

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

        let result = result_info(graph, tc_id);
        let size_str = result
            .as_ref()
            .map(|r| format!("→ {}", format_result_size(r.char_len)));
        let (tool_name, tool_args) = arguments.display_parts();
        lines.push(format_tool_line(&ToolLineParams {
            icon,
            icon_color,
            tool_name,
            tool_args: &tool_args,
            size_str: size_str.as_deref().unwrap_or(""),
            duration: &duration,
            is_active,
            width,
        }));

        if expanded {
            if let Some(ref info) = result {
                append_result_content(&mut lines, &info.text, info.is_error, width);
            }
        }
    }
    lines
}

struct ResultInfo {
    text: String,
    char_len: usize,
    is_error: bool,
}

fn result_info(graph: &ConversationGraph, tool_call_id: Uuid) -> Option<ResultInfo> {
    let ids = graph.sources_by_edge(tool_call_id, EdgeKind::Produced);
    let r_id = ids.first()?;
    if let Some(Node::ToolResult {
        content, is_error, ..
    }) = graph.node(*r_id)
    {
        let len = content.char_len();
        if len > 0 {
            return Some(ResultInfo {
                text: content.text_content().to_string(),
                char_len: len,
                is_error: *is_error,
            });
        }
    }
    None
}

fn format_result_size(chars: usize) -> String {
    if chars < 1000 {
        format!("{chars}")
    } else if chars < 10_000 {
        // Precision loss acceptable — display-only approximation.
        #[allow(clippy::cast_precision_loss)]
        let kb = chars as f64 / 1000.0;
        format!("{kb:.1}K")
    } else {
        format!("{}K", chars / 1000)
    }
}

fn append_result_content(lines: &mut Vec<Line<'static>>, text: &str, is_error: bool, width: usize) {
    let color = if is_error {
        Color::Red
    } else {
        Color::DarkGray
    };
    let content_width = width.saturating_sub(4);
    let source: Vec<&str> = text.lines().take(MAX_RESULT_LINES).collect();
    let total = text.lines().count();

    let border = "─".repeat(content_width.min(width.saturating_sub(4)));
    lines.push(Line::from(Span::styled(
        format!("  ┌─{border}"),
        Style::default().fg(color),
    )));
    for line in &source {
        lines.push(Line::from(vec![
            Span::styled("  │ ", Style::default().fg(color)),
            Span::styled(
                truncate(line, content_width),
                Style::default().fg(Color::White),
            ),
        ]));
    }
    if total > MAX_RESULT_LINES {
        lines.push(Line::from(Span::styled(
            format!("  │ [... {} more lines]", total - MAX_RESULT_LINES),
            Style::default().fg(Color::DarkGray),
        )));
    }
    lines.push(Line::from(Span::styled(
        format!("  └─{border}"),
        Style::default().fg(color),
    )));
}

/// Parameters for rendering a single tool status line.
struct ToolLineParams<'a> {
    icon: &'a str,
    icon_color: Color,
    tool_name: &'a str,
    tool_args: &'a str,
    size_str: &'a str,
    duration: &'a TaskDuration,
    is_active: bool,
    width: usize,
}

/// Render a tool status line: icon + name (Magenta) + args (White) + size + duration.
fn format_tool_line(p: &ToolLineParams<'_>) -> Line<'static> {
    let dur = format_duration(p.duration);
    let dur_color = if p.is_active {
        Color::Yellow
    } else {
        Color::DarkGray
    };
    let size_w = if p.size_str.is_empty() {
        0
    } else {
        p.size_str.len() + 1
    };
    let fixed = 2 + 1 + size_w + dur.len();
    let budget = p.width.saturating_sub(fixed);
    let sep = if p.tool_args.is_empty() { "" } else { " " };
    let args_budget = budget.saturating_sub(p.tool_name.len() + sep.len());
    let args = truncate(p.tool_args, args_budget);
    let used = p.tool_name.len() + sep.len() + visible_width(&args);
    let padding = budget.saturating_sub(used);

    let mut spans = vec![
        Span::styled(format!("{} ", p.icon), Style::default().fg(p.icon_color)),
        Span::styled(
            p.tool_name.to_string(),
            Style::default().fg(Color::Magenta).bold(),
        ),
    ];
    if !p.tool_args.is_empty() {
        spans.push(Span::styled(sep, Style::default().fg(Color::DarkGray)));
        spans.push(Span::styled(args, Style::default().fg(Color::White)));
    }
    spans.push(Span::raw(" ".repeat(padding)));
    if !p.size_str.is_empty() {
        spans.push(Span::styled(
            format!("{} ", p.size_str),
            Style::default().fg(Color::DarkGray),
        ));
    }
    spans.push(Span::styled(dur, Style::default().fg(dur_color)));
    Line::from(spans)
}
