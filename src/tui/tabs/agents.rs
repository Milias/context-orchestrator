//! Agent widget library: reusable rendering functions for agent status.
//!
//! Provides building blocks for the overview tab: agent card and running tasks.
//! No top-level render function; the overview tab composes these widgets directly.

use crate::graph::tool_types::ToolCallStatus;
use crate::graph::{BackgroundTaskKind, ConversationGraph, Node, TaskStatus};
use crate::tui::widgets::tool_status::{
    elapsed, finished, format_duration, tool_call_status_icon, truncate,
};
use crate::tui::{TuiState, SPINNER_FRAMES};

use chrono::Utc;
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Paragraph};

/// Compute the agent card height based on the number of active agents.
/// Each agent gets 2 lines (phase + detail); borders add 2.
pub(super) fn agent_card_height(tui_state: &TuiState) -> u16 {
    let count = tui_state.agent_displays.len();
    if count == 0 {
        return 3; // border + "(idle)" + border
    }
    // border(2) + 2 lines per agent + (count-1) spacing lines.
    let inner = 2 * count + count.saturating_sub(1);
    // Clamp: u16 bounded by small agent counts.
    #[allow(clippy::cast_possible_truncation)] // Justified: agent count is small (<10).
    let h = (inner as u16).saturating_add(2);
    h.max(5)
}

/// Render the agent status card showing all active agents.
pub(super) fn render_agent_card(frame: &mut Frame, area: Rect, tui_state: &TuiState) {
    let block = Block::default().title("Agents").borders(Borders::ALL);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.height == 0 || inner.width < 4 {
        return;
    }

    if tui_state.agent_displays.is_empty() {
        let idle = Paragraph::new(Span::styled("(idle)", Style::default().fg(Color::DarkGray)));
        frame.render_widget(idle, inner);
        return;
    }

    let mut lines: Vec<Line<'_>> = Vec::new();
    let phase_text = tui_state.status_message.as_deref().unwrap_or("Working...");

    for (idx, (agent_id, display)) in tui_state.agent_displays.iter().enumerate() {
        if idx > 0 {
            lines.push(Line::raw(""));
        }
        let spinner = display.spinner_char();
        let short_id = &agent_id.to_string()[..8];
        lines.push(Line::from(vec![
            Span::styled(format!("{spinner} "), Style::default().fg(Color::Yellow)),
            Span::styled(
                format!("Agent {short_id}"),
                Style::default().fg(Color::Cyan).bold(),
            ),
            Span::styled(
                format!("  {phase_text}"),
                Style::default().fg(Color::DarkGray),
            ),
        ]));

        render_agent_detail(&display.phase, inner.width, &mut lines);
    }

    let text = Text::from(lines);
    frame.render_widget(Paragraph::new(text), inner);
}

/// Append a streaming preview or phase detail line for one agent.
fn render_agent_detail(
    phase: &crate::tui::AgentVisualPhase,
    width: u16,
    lines: &mut Vec<Line<'_>>,
) {
    match phase {
        crate::tui::AgentVisualPhase::Streaming { text, is_thinking } => {
            let label = if *is_thinking { "thinking" } else { "writing" };
            let preview = truncate(
                text.lines().next_back().unwrap_or(""),
                width.saturating_sub(4) as usize,
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
}

/// Count active items for sizing the Running section.
pub(super) fn running_tasks_height(graph: &ConversationGraph) -> u16 {
    let count = count_running(graph);
    if count == 0 {
        return 0; // Hide the section entirely when nothing is running.
    }
    // border (2) + items. Clamp to 10 rows max.
    let n: u16 = u16::try_from(count).unwrap_or(u16::MAX);
    n.saturating_add(2).min(10)
}

/// Count running background tasks (non-AgentPhase) + active tool calls.
pub(super) fn count_running(graph: &ConversationGraph) -> usize {
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
pub(super) fn render_running_tasks(
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
        .agent_displays
        .values()
        .next()
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
                Span::styled(name, Style::default().fg(Color::Blue)),
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
            let (tool_name, tool_args) = arguments.display_parts();
            let budget = width.saturating_sub(4 + dur.len());
            let full = format!("{tool_name} {tool_args}");
            let truncated = truncate(&full, budget);
            // Color: tool name in Magenta+bold, args in White.
            let name_len = tool_name.len().min(truncated.chars().count());
            let pad = budget.saturating_sub(truncated.chars().count());
            let name_part: String = truncated.chars().take(name_len).collect();
            let args_part: String = truncated.chars().skip(name_len).collect();
            lines.push(Line::from(vec![
                Span::styled(format!("{icon} "), Style::default().fg(color)),
                Span::styled(name_part, Style::default().fg(Color::Magenta).bold()),
                Span::styled(args_part, Style::default().fg(Color::White)),
                Span::raw(" ".repeat(pad)),
                Span::styled(dur, Style::default().fg(Color::DarkGray)),
            ]));
        }
    }

    let max_rows = inner.height as usize;
    lines.truncate(max_rows);
    frame.render_widget(Paragraph::new(Text::from(lines)), inner);
}

/// Render recently completed/failed tool calls, sorted newest first.
pub(super) fn render_recent_completions(
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
    let w = inner.width as usize;
    let max_rows = inner.height as usize;
    let mut calls: Vec<&Node> = graph.nodes_by(|n| {
        matches!(
            n,
            Node::ToolCall {
                status: ToolCallStatus::Completed | ToolCallStatus::Failed,
                ..
            }
        )
    });
    calls.sort_by_key(|n| std::cmp::Reverse(n.created_at()));
    // Cast safety: bounded by call count, well within u16.
    #[allow(clippy::cast_possible_truncation)] // Justified: max_offset ≤ calls.len().
    let max_offset = calls.len().saturating_sub(max_rows) as u16;
    tui_state.recent_max = max_offset;
    tui_state.recent_scroll.apply_max(max_offset);
    let offset = tui_state.recent_scroll.position() as usize;
    let calls: Vec<_> = calls.into_iter().skip(offset).take(max_rows).collect();

    let lines: Vec<Line<'_>> = calls
        .iter()
        .filter_map(|n| {
            let Node::ToolCall {
                status,
                arguments,
                created_at,
                completed_at,
                ..
            } = n
            else {
                return None;
            };
            let (icon, color) = tool_call_status_icon(status);
            let dur = format_duration(&match completed_at {
                Some(end) => finished(*end, *created_at),
                None => elapsed(now, *created_at),
            });
            let fixed = 2 + 1 + dur.len();
            let (tool_name, tool_args) = arguments.display_parts();
            let budget = w.saturating_sub(fixed);
            let full = format!("{tool_name} {tool_args}");
            let trunc = truncate(&full, budget);
            let nl = tool_name.len().min(trunc.chars().count());
            let np: String = trunc.chars().take(nl).collect();
            let ap: String = trunc.chars().skip(nl).collect();
            let pad = budget.saturating_sub(trunc.chars().count());
            Some(Line::from(vec![
                Span::styled(format!("{icon} "), Style::default().fg(color)),
                Span::styled(np, Style::default().fg(Color::Magenta).bold()),
                Span::styled(ap, Style::default().fg(Color::White)),
                Span::raw(" ".repeat(pad)),
                Span::styled(dur, Style::default().fg(Color::DarkGray)),
            ]))
        })
        .collect();

    if lines.is_empty() {
        let empty = Span::styled("(no completions)", Style::default().fg(Color::DarkGray));
        frame.render_widget(Paragraph::new(empty), inner);
    } else {
        frame.render_widget(Paragraph::new(Text::from(lines)), inner);
    }
}
