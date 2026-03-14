//! Overview dashboard tab: real-time operational dashboard.
//!
//! Stacked sections showing live system state: agents, active work,
//! running tasks/tools, pending questions, and stats. Empty sections
//! collapse to zero height so the dashboard adapts to current activity.

use crate::graph::node::{QuestionStatus, WorkItemStatus};
use crate::graph::{ConversationGraph, EdgeKind, Node};
use crate::tui::widgets::tool_status::truncate;
use crate::tui::TuiState;

use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Paragraph};

use super::{agents, work};

// ── Section height computation ──────────────────────────────────────

/// Compute the number of active work items (status == Active) with their
/// active children, for sizing the Active Work section.
fn active_work_height(graph: &ConversationGraph) -> u16 {
    let mut count: usize = 0;
    for node in graph.nodes_by(|n| {
        matches!(
            n,
            Node::WorkItem {
                status: WorkItemStatus::Active,
                ..
            }
        )
    }) {
        count += 1; // The work item itself.
        // Count active children (one level deep).
        let children = graph.children_of(node.id());
        for cid in children {
            if let Some(Node::WorkItem {
                status: WorkItemStatus::Active,
                ..
            }) = graph.node(cid)
            {
                count += 1;
            }
        }
    }
    if count == 0 {
        return 0;
    }
    // Borders (2) + items.
    let n = u16::try_from(count).unwrap_or(u16::MAX);
    n.saturating_add(2).min(12)
}

/// Collect question nodes that are not in a terminal-resolved state.
/// Returns `(question_line_count, questions)` for sizing and rendering.
fn collect_visible_questions(graph: &ConversationGraph) -> Vec<&Node> {
    let mut questions: Vec<&Node> = graph.nodes_by(|n| {
        matches!(
            n,
            Node::Question {
                status: QuestionStatus::Pending
                    | QuestionStatus::Claimed
                    | QuestionStatus::PendingApproval,
                ..
            }
        )
    });
    // Also show recently answered questions (last 3).
    let mut answered: Vec<&Node> = graph.nodes_by(|n| {
        matches!(
            n,
            Node::Question {
                status: QuestionStatus::Answered,
                ..
            }
        )
    });
    answered.sort_by_key(|n| std::cmp::Reverse(n.created_at()));
    answered.truncate(3);
    questions.extend(answered);
    // Sort all: pending first, then by creation time (newest first).
    questions.sort_by(|a, b| {
        let priority = |n: &Node| -> u8 {
            match n {
                Node::Question {
                    status: QuestionStatus::Pending,
                    ..
                } => 0,
                Node::Question {
                    status: QuestionStatus::Claimed,
                    ..
                } => 1,
                Node::Question {
                    status: QuestionStatus::PendingApproval,
                    ..
                } => 2,
                _ => 3,
            }
        };
        priority(a)
            .cmp(&priority(b))
            .then(b.created_at().cmp(&a.created_at()))
    });
    questions
}

/// Compute the questions section height: each question + its answers.
fn questions_height(graph: &ConversationGraph) -> u16 {
    let questions = collect_visible_questions(graph);
    if questions.is_empty() {
        return 0;
    }
    let mut lines: usize = 0;
    for q in &questions {
        lines += 1; // The question line.
        let answers = graph.sources_by_edge(q.id(), EdgeKind::Answers);
        lines += answers.len();
    }
    let n = u16::try_from(lines).unwrap_or(u16::MAX);
    n.saturating_add(2).min(15) // borders + cap
}

/// Fixed height for the stats section (always visible).
const STATS_HEIGHT: u16 = 9;

// ── Main render ─────────────────────────────────────────────────────

/// Render the Overview dashboard tab.
///
/// Stacked sections that auto-size based on content:
/// Agents | Active Work | Running | Questions | Stats.
/// Empty sections collapse to zero height.
pub fn render(
    frame: &mut Frame,
    area: Rect,
    graph: &ConversationGraph,
    tui_state: &mut TuiState,
) {
    let agent_h = agents::agent_card_height(tui_state);
    let work_h = active_work_height(graph);
    let running_h = agents::running_tasks_height(graph);
    let questions_h = questions_height(graph);

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(agent_h),
            Constraint::Length(work_h),
            Constraint::Length(running_h),
            Constraint::Length(questions_h),
            Constraint::Length(STATS_HEIGHT),
            Constraint::Min(0), // absorb remaining space
        ])
        .split(area);

    agents::render_agent_card(frame, rows[0], tui_state);
    render_active_work(frame, rows[1], graph);
    agents::render_running_tasks(frame, rows[2], graph, tui_state);
    render_questions(frame, rows[3], graph);
    render_stats(frame, rows[4], graph, tui_state);
}

// ── Active Work section ─────────────────────────────────────────────

/// Render the Active Work section: only work items with `Active` status
/// and their active children, shown as a flat indented list.
fn render_active_work(frame: &mut Frame, area: Rect, graph: &ConversationGraph) {
    if area.height < 3 {
        return;
    }
    let block = Block::default()
        .title("Active Work")
        .borders(Borders::ALL);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.height == 0 || inner.width < 8 {
        return;
    }

    let tree = work::build_work_tree(graph);
    let active_items: Vec<_> = tree
        .iter()
        .filter(|item| item.status == WorkItemStatus::Active)
        .collect();

    if active_items.is_empty() {
        return;
    }

    let width = inner.width as usize;
    let max_lines = inner.height as usize;
    let mut lines: Vec<Line<'_>> = Vec::new();

    for item in active_items {
        if lines.len() >= max_lines {
            break;
        }
        // Render root item without selection highlight (overview has no selection).
        work::render_item(&mut lines, item, 0, width, max_lines, usize::MAX);
    }

    lines.truncate(max_lines);
    frame.render_widget(Paragraph::new(Text::from(lines)), inner);
}

// ── Questions section ───────────────────────────────────────────────

/// Status label for a question.
fn question_status_label(status: QuestionStatus) -> &'static str {
    match status {
        QuestionStatus::Pending => "Pending",
        QuestionStatus::Claimed => "Claimed",
        QuestionStatus::PendingApproval => "Approval",
        QuestionStatus::Answered => "Answered",
        QuestionStatus::Rejected => "Rejected",
        QuestionStatus::TimedOut => "Timed Out",
    }
}

/// Color for a question status indicator.
fn question_status_color(status: QuestionStatus) -> Color {
    match status {
        QuestionStatus::Pending => Color::Yellow,
        QuestionStatus::Claimed => Color::Cyan,
        QuestionStatus::PendingApproval => Color::Magenta,
        QuestionStatus::Answered => Color::Green,
        QuestionStatus::Rejected => Color::Red,
        QuestionStatus::TimedOut => Color::DarkGray,
    }
}

/// Render the Questions section: pending/active questions and their answers.
fn render_questions(frame: &mut Frame, area: Rect, graph: &ConversationGraph) {
    if area.height < 3 {
        return;
    }
    let block = Block::default()
        .title("Questions")
        .borders(Borders::ALL);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.height == 0 || inner.width < 8 {
        return;
    }

    let questions = collect_visible_questions(graph);
    if questions.is_empty() {
        return;
    }

    let width = inner.width as usize;
    let max_lines = inner.height as usize;
    let mut lines: Vec<Line<'_>> = Vec::new();

    for node in &questions {
        if lines.len() >= max_lines {
            break;
        }
        let Node::Question {
            id,
            content,
            status,
            ..
        } = node
        else {
            continue;
        };

        // Question line: ? "content preview" [Status]
        let label = question_status_label(*status);
        let color = question_status_color(*status);
        // Budget: "? " (2) + quotes (2) + " [" + label + "]" (3 + label.len).
        let fixed = 2 + 2 + 2 + label.len() + 1;
        let content_budget = width.saturating_sub(fixed);
        let preview = truncate(content, content_budget);

        lines.push(Line::from(vec![
            Span::styled("? ", Style::default().fg(Color::Yellow)),
            Span::styled(
                format!("\"{preview}\""),
                Style::default().fg(Color::White),
            ),
            Span::styled(
                format!(" [{label}]"),
                Style::default().fg(color),
            ),
        ]));

        // Answers for this question.
        let answer_ids = graph.sources_by_edge(*id, EdgeKind::Answers);
        for aid in answer_ids {
            if lines.len() >= max_lines {
                break;
            }
            if let Some(Node::Answer { content, .. }) = graph.node(aid) {
                // Answer line: "  A content_preview"
                let answer_budget = width.saturating_sub(4); // "  A " prefix
                let answer_preview = truncate(content, answer_budget);
                lines.push(Line::from(vec![
                    Span::styled("  A ", Style::default().fg(Color::Green)),
                    Span::styled(answer_preview, Style::default().fg(Color::White)),
                ]));
            }
        }
    }

    lines.truncate(max_lines);
    frame.render_widget(Paragraph::new(Text::from(lines)), inner);
}

// ── Stats section ───────────────────────────────────────────────────

/// Render the Stats section using the shared stats panel widget.
fn render_stats(
    frame: &mut Frame,
    area: Rect,
    graph: &ConversationGraph,
    tui_state: &TuiState,
) {
    crate::tui::widgets::stats_panel::render(frame, area, graph, tui_state);
}
