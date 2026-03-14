//! Agents tab: primary monitoring view.
//!
//! Shows the current agent's status, recent tool call completions,
//! attention items (errors), and basic stats. When the conversation
//! panel is hidden, uses a 3-column layout; otherwise a stacked layout.

use crate::graph::tool_types::ToolCallStatus;
use crate::graph::{BackgroundTaskKind, ConversationGraph, Node, TaskStatus};
use crate::tui::widgets::tool_status::{
    elapsed, finished, format_duration, tool_call_status_icon, truncate,
};
use crate::tui::{TuiState, SPINNER_FRAMES};

use chrono::Utc;
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Paragraph};

/// Render the Agents tab content into the given area.
pub fn render(frame: &mut Frame, area: Rect, graph: &ConversationGraph, tui_state: &mut TuiState) {
    if area.width < 40 {
        render_compact(frame, area, graph, tui_state);
    } else {
        render_standard(frame, area, graph, tui_state);
    }
}

/// Standard layout: agent card + running tasks + recent completions + stats.
fn render_standard(
    frame: &mut Frame,
    area: Rect,
    graph: &ConversationGraph,
    tui_state: &mut TuiState,
) {
    let running_h = running_tasks_height(graph);
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(agent_card_height(tui_state)),
            Constraint::Length(running_h),
            Constraint::Min(3),
        ])
        .split(area);

    render_agent_card(frame, vertical[0], tui_state);
    render_running_tasks(frame, vertical[1], graph, tui_state);
    render_recent_completions(frame, vertical[2], graph, tui_state);
}

/// Compact layout for narrow terminals: everything stacked.
fn render_compact(
    frame: &mut Frame,
    area: Rect,
    graph: &ConversationGraph,
    tui_state: &mut TuiState,
) {
    let running_h = running_tasks_height(graph);
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(agent_card_height(tui_state)),
            Constraint::Length(running_h),
            Constraint::Min(3),
        ])
        .split(area);

    render_agent_card(frame, vertical[0], tui_state);
    render_running_tasks(frame, vertical[1], graph, tui_state);
    render_recent_completions(frame, vertical[2], graph, tui_state);
}

/// Compute the agent card height based on the number of active tool calls.
fn agent_card_height(tui_state: &TuiState) -> u16 {
    match &tui_state.agent_display {
        Some(_) => 5, // border + phase line + detail line + padding + border
        None => 3,    // border + "(idle)" + border
    }
}

/// Render the agent status card.
fn render_agent_card(frame: &mut Frame, area: Rect, tui_state: &TuiState) {
    let block = Block::default().title("Agents").borders(Borders::ALL);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.height == 0 || inner.width < 4 {
        return;
    }

    let Some(display) = &tui_state.agent_display else {
        let idle = Paragraph::new(Span::styled("(idle)", Style::default().fg(Color::DarkGray)));
        frame.render_widget(idle, inner);
        return;
    };

    let spinner = display.spinner_char();
    let mut lines: Vec<Line<'_>> = Vec::new();

    // Phase line.
    let phase_text = tui_state.status_message.as_deref().unwrap_or("Working...");
    lines.push(Line::from(vec![
        Span::styled(format!("{spinner} "), Style::default().fg(Color::Yellow)),
        Span::styled("Agent #1", Style::default().fg(Color::White).bold()),
        Span::styled(
            format!("  {phase_text}"),
            Style::default().fg(Color::DarkGray),
        ),
    ]));

    // Streaming preview or phase detail.
    match &display.phase {
        crate::tui::AgentVisualPhase::Streaming { text, is_thinking } => {
            let label = if *is_thinking { "thinking" } else { "writing" };
            let preview = truncate(
                text.lines().next_back().unwrap_or(""),
                inner.width.saturating_sub(4) as usize,
            );
            lines.push(Line::from(vec![
                Span::styled(
                    format!("  [{label}] "),
                    Style::default().fg(Color::DarkGray),
                ),
                Span::styled(preview, Style::default().fg(Color::White)),
            ]));
        }
        crate::tui::AgentVisualPhase::ExecutingTools => {
            lines.push(Line::from(Span::styled(
                "  Running tool calls...",
                Style::default().fg(Color::DarkGray),
            )));
        }
        crate::tui::AgentVisualPhase::Preparing => {}
    }

    let text = Text::from(lines);
    frame.render_widget(Paragraph::new(text), inner);
}

/// Count active items for sizing the Running section.
fn running_tasks_height(graph: &ConversationGraph) -> u16 {
    let count = count_running(graph);
    if count == 0 {
        return 0; // Hide the section entirely when nothing is running.
    }
    // border (2) + items. Clamp to 10 rows max.
    let n: u16 = u16::try_from(count).unwrap_or(u16::MAX);
    n.saturating_add(2).min(10)
}

/// Count running background tasks (non-AgentPhase) + active tool calls.
fn count_running(graph: &ConversationGraph) -> usize {
    let bg = graph
        .nodes_by(|n| {
            matches!(
                n,
                Node::BackgroundTask {
                    status: TaskStatus::Running,
                    kind,
                    ..
                } if *kind != BackgroundTaskKind::AgentPhase
            )
        })
        .len();
    let tc = graph
        .nodes_by(|n| {
            matches!(
                n,
                Node::ToolCall {
                    status: ToolCallStatus::Pending | ToolCallStatus::Running,
                    ..
                }
            )
        })
        .len();
    bg + tc
}

/// Render active background tasks and tool calls.
fn render_running_tasks(
    frame: &mut Frame,
    area: Rect,
    graph: &ConversationGraph,
    tui_state: &TuiState,
) {
    if area.height < 3 {
        return;
    }
    let block = Block::default().title("Running").borders(Borders::ALL);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.height == 0 || inner.width < 8 {
        return;
    }

    let now = Utc::now();
    let width = inner.width as usize;
    let spinner_tick = tui_state
        .agent_display
        .as_ref()
        .map_or(0, |d| d.spinner_tick);
    let spinner = SPINNER_FRAMES[spinner_tick % SPINNER_FRAMES.len()];

    let mut lines: Vec<Line<'_>> = Vec::new();

    // Background tasks (non-AgentPhase, Running).
    for node in graph.nodes_by(|n| {
        matches!(
            n,
            Node::BackgroundTask {
                status: TaskStatus::Running,
                kind,
                ..
            } if *kind != BackgroundTaskKind::AgentPhase
        )
    }) {
        if let Node::BackgroundTask {
            description,
            created_at,
            ..
        } = node
        {
            let dur = format_duration(&elapsed(now, *created_at));
            let name = truncate(description, width.saturating_sub(4 + dur.len()));
            let pad = width.saturating_sub(2 + name.chars().count() + 1 + dur.len());
            lines.push(Line::from(vec![
                Span::styled(format!("{spinner} "), Style::default().fg(Color::Cyan)),
                Span::styled(name, Style::default().fg(Color::White)),
                Span::raw(" ".repeat(pad)),
                Span::styled(dur, Style::default().fg(Color::DarkGray)),
            ]));
        }
    }

    // Active tool calls (Pending/Running).
    for node in graph.nodes_by(|n| {
        matches!(
            n,
            Node::ToolCall {
                status: ToolCallStatus::Pending | ToolCallStatus::Running,
                ..
            }
        )
    }) {
        if let Node::ToolCall {
            status,
            arguments,
            created_at,
            ..
        } = node
        {
            let (icon, color) = if *status == ToolCallStatus::Running {
                (spinner, Color::Yellow)
            } else {
                tool_call_status_icon(status)
            };
            let dur = format_duration(&elapsed(now, *created_at));
            let summary = arguments.display_summary();
            let name = truncate(&summary, width.saturating_sub(4 + dur.len()));
            let pad = width.saturating_sub(2 + name.chars().count() + 1 + dur.len());
            lines.push(Line::from(vec![
                Span::styled(format!("{icon} "), Style::default().fg(color)),
                Span::styled(name, Style::default().fg(Color::Magenta)),
                Span::raw(" ".repeat(pad)),
                Span::styled(dur, Style::default().fg(Color::DarkGray)),
            ]));
        }
    }

    let max_rows = inner.height as usize;
    lines.truncate(max_rows);
    frame.render_widget(Paragraph::new(Text::from(lines)), inner);
}

/// Render a list of recent tool call completions from the graph.
fn render_recent_completions(
    frame: &mut Frame,
    area: Rect,
    graph: &ConversationGraph,
    tui_state: &mut TuiState,
) {
    let block = Block::default().title("Recent").borders(Borders::ALL);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.height == 0 || inner.width < 8 {
        return;
    }

    let now = Utc::now();
    let width = inner.width as usize;
    let max_items = inner.height as usize;

    // Collect recent tool calls (completed/failed), sorted newest first.
    let mut tool_calls: Vec<&Node> = graph.nodes_by(|n| {
        matches!(
            n,
            Node::ToolCall {
                status: ToolCallStatus::Completed | ToolCallStatus::Failed,
                ..
            }
        )
    });
    tool_calls.sort_by_key(|n| std::cmp::Reverse(n.created_at()));

    // Publish total and clamp scroll.
    tui_state.agents_total = tool_calls.len();
    let max_offset = tool_calls.len().saturating_sub(max_items);
    tui_state.agents_scroll = tui_state.agents_scroll.min(max_offset);

    // Apply scroll window.
    let tool_calls: Vec<_> = tool_calls
        .into_iter()
        .skip(tui_state.agents_scroll)
        .take(max_items)
        .collect();

    let mut lines: Vec<Line<'_>> = Vec::new();
    for node in tool_calls {
        if let Node::ToolCall {
            status,
            arguments,
            created_at,
            completed_at,
            ..
        } = node
        {
            let (icon, color) = tool_call_status_icon(status);
            let duration = match completed_at {
                Some(end) => finished(*end, *created_at),
                None => elapsed(now, *created_at),
            };
            let dur_str = format_duration(&duration);
            let name = arguments.display_summary();
            let fixed = 2 + 1 + dur_str.len(); // icon + space + padding + duration
            let name_budget = width.saturating_sub(fixed);
            let name = truncate(&name, name_budget);
            let padding = name_budget.saturating_sub(name.chars().count());

            lines.push(Line::from(vec![
                Span::styled(format!("{icon} "), Style::default().fg(color)),
                Span::styled(name, Style::default().fg(Color::White)),
                Span::raw(" ".repeat(padding)),
                Span::styled(dur_str, Style::default().fg(Color::DarkGray)),
            ]));
        }
    }

    if lines.is_empty() {
        lines.push(Line::from(Span::styled(
            "(no recent activity)",
            Style::default().fg(Color::DarkGray),
        )));
    }

    frame.render_widget(Paragraph::new(Text::from(lines)), inner);
}

