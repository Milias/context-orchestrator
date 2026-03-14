//! Q&A tree for the Graph tab: questions and their answers.
//!
//! Renders questions as tree roots sorted by lifecycle status (open first,
//! then answered, then terminal). Answers appear as children connected
//! via `Answers` edges. Uses [`TreePrefix`] for tree-command-style
//! connectors and [`ExplorerState`] for selection and collapse.

use uuid::Uuid;

use crate::graph::node::{QuestionDestination, QuestionStatus};
use crate::graph::{ConversationGraph, EdgeKind, Node};
use crate::tui::state::{ExplorerFocus, GraphSection};
use crate::tui::tabs::explorer::ExplorerState;
use crate::tui::tabs::graph::tree_lines::TreePrefix;
use crate::tui::widgets::tool_status::truncate;
use crate::tui::TuiState;

use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Paragraph};

/// Icon glyph for question nodes.
const QUESTION_ICON: &str = "?";
/// Icon glyph for answer nodes.
const ANSWER_ICON: &str = "A";

/// A flattened Q&A tree node carrying its tree prefix and display data.
struct FlatItem {
    /// Graph node UUID for selection mapping and detail panel.
    id: Uuid,
    /// Pre-rendered tree connector prefix (e.g. `"├── "`).
    prefix: String,
    /// First line of content text, pre-truncated.
    content: String,
    /// Whether this is a question or answer row.
    kind: QaKind,
}

/// Discriminates question rows from answer rows for rendering.
enum QaKind {
    /// A question root with its status and routing destination.
    Question {
        status: QuestionStatus,
        destination: QuestionDestination,
        has_answers: bool,
        is_collapsed: bool,
    },
    /// An answer child.
    Answer,
}

/// Build and render the Q&A tree with tree-command-style connectors.
///
/// Returns the UUID of the currently selected node (if any) so the
/// caller can pass it to the detail panel.
pub fn render(
    frame: &mut Frame,
    tree_area: Rect,
    graph: &ConversationGraph,
    tui_state: &mut TuiState,
) -> Option<Uuid> {
    let focused = tui_state
        .explorer
        .get(&GraphSection::QA)
        .is_none_or(|e| e.focus == ExplorerFocus::Tree);
    let border_style = if focused {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    };
    let block = Block::default()
        .title("Q&A")
        .borders(Borders::ALL)
        .border_style(border_style);
    let inner = block.inner(tree_area);
    frame.render_widget(block, tree_area);

    if inner.height == 0 || inner.width < 10 {
        return None;
    }

    let explorer = tui_state
        .explorer
        .get_mut(&GraphSection::QA)
        .expect("QA explorer state must exist");

    let flat_items = build_flat_tree(graph, explorer);

    if flat_items.is_empty() {
        let empty = Paragraph::new(Span::styled(
            "(no questions)",
            Style::default().fg(Color::DarkGray),
        ));
        frame.render_widget(empty, inner);
        explorer.visible_count = 0;
        return None;
    }

    // Publish visible count so input handler can clamp selection.
    explorer.visible_count = flat_items.len();
    if explorer.selected >= flat_items.len() {
        explorer.selected = flat_items.len().saturating_sub(1);
    }

    let selected_id = flat_items.get(explorer.selected).map(|item| item.id);
    let selected_idx = explorer.selected;
    let width = inner.width as usize;
    let max_lines = inner.height as usize;

    let lines: Vec<Line<'_>> = flat_items
        .iter()
        .take(max_lines)
        .enumerate()
        .map(|(i, item)| render_flat_item(item, i, selected_idx, width))
        .collect();

    frame.render_widget(Paragraph::new(Text::from(lines)), inner);

    selected_id
}

/// Build a flattened list of Q&A items in sorted tree-walk order.
///
/// Questions are roots sorted by status group (open first, then
/// answered, then terminal). Within each group, newest questions
/// appear first. Answers are children of their parent question.
fn build_flat_tree(graph: &ConversationGraph, explorer: &ExplorerState) -> Vec<FlatItem> {
    let mut questions: Vec<&Node> = graph.nodes_by(|n| matches!(n, Node::Question { .. }));

    // Sort: open statuses first, then answered, then terminal. Within groups, newest first.
    questions.sort_by(|a, b| {
        status_sort_key(a)
            .cmp(&status_sort_key(b))
            .then_with(|| b.created_at().cmp(&a.created_at()))
    });

    let mut flat = Vec::new();
    let total = questions.len();

    for (i, question) in questions.iter().enumerate() {
        let is_last = i + 1 == total;
        flatten_question(graph, explorer, question, is_last, &mut flat);
    }
    flat
}

/// Flatten a question and its visible answer children into the flat list.
fn flatten_question(
    graph: &ConversationGraph,
    explorer: &ExplorerState,
    question: &Node,
    is_last_sibling: bool,
    out: &mut Vec<FlatItem>,
) {
    let Node::Question {
        id,
        content,
        status,
        destination,
        ..
    } = question
    else {
        return;
    };

    let answer_ids = graph.sources_by_edge(*id, EdgeKind::Answers);
    let mut answers: Vec<&Node> = answer_ids
        .iter()
        .filter_map(|aid| graph.node(*aid))
        .collect();
    // Sort answers by creation time (oldest first so conversation reads top-down).
    answers.sort_by_key(|a| a.created_at());

    let has_answers = !answers.is_empty();
    let is_collapsed = explorer.is_collapsed(id);

    let first_line = content.lines().next().unwrap_or("");
    let root_prefix = TreePrefix::new();

    out.push(FlatItem {
        id: *id,
        prefix: root_prefix.render(is_last_sibling),
        content: first_line.to_string(),
        kind: QaKind::Question {
            status: *status,
            destination: *destination,
            has_answers,
            is_collapsed,
        },
    });

    // Append answer children if not collapsed.
    if has_answers && !is_collapsed {
        let child_prefix = root_prefix.child(is_last_sibling);
        let total_answers = answers.len();
        for (j, answer) in answers.iter().enumerate() {
            let answer_is_last = j + 1 == total_answers;
            let answer_first_line = answer.content().lines().next().unwrap_or("");
            out.push(FlatItem {
                id: answer.id(),
                prefix: child_prefix.render(answer_is_last),
                content: answer_first_line.to_string(),
                kind: QaKind::Answer,
            });
        }
    }
}

/// Render a single flat item as a styled `Line`.
fn render_flat_item(
    item: &FlatItem,
    line_idx: usize,
    selected_idx: usize,
    width: usize,
) -> Line<'static> {
    let is_selected = line_idx == selected_idx;
    let dim = Style::default().fg(Color::DarkGray);
    let mut spans = Vec::new();

    spans.push(Span::styled(item.prefix.clone(), dim));

    match &item.kind {
        QaKind::Question {
            status,
            destination,
            has_answers,
            is_collapsed,
        } => {
            render_question_spans(
                &mut spans,
                &item.content,
                *status,
                *destination,
                *has_answers,
                *is_collapsed,
                width,
            );
        }
        QaKind::Answer => {
            render_answer_spans(&mut spans, &item.content, width);
        }
    }

    // Apply background highlight to all spans on the selected line.
    if is_selected {
        let bg = Color::Rgb(40, 40, 60);
        for span in &mut spans {
            span.style = span.style.bg(bg);
        }
    }

    Line::from(spans)
}

/// Append spans for a question row: collapse indicator, icon, content, status badge, destination.
fn render_question_spans(
    spans: &mut Vec<Span<'static>>,
    content: &str,
    status: QuestionStatus,
    destination: QuestionDestination,
    has_answers: bool,
    is_collapsed: bool,
    width: usize,
) {
    let dim = Style::default().fg(Color::DarkGray);

    // Collapse/expand indicator.
    if has_answers {
        let indicator = if is_collapsed {
            "\u{25b6} " // ▶
        } else {
            "\u{25bc} " // ▼
        };
        spans.push(Span::styled(indicator, dim));
    }

    // Question icon.
    spans.push(Span::styled(
        format!("{QUESTION_ICON} "),
        Style::default().fg(Color::Yellow),
    ));

    // Status badge.
    let (badge_text, badge_color) = status_badge(status);

    // Destination badge.
    let dest_text = destination_badge(destination);

    // Compute content budget: total width minus prefix, icon, badges, and padding.
    let overhead = 2 // "? "
        + badge_text.len() + 2 // " [badge]"
        + dest_text.len() + 1 // " (→dest)"
        + if has_answers { 2 } else { 0 }; // collapse indicator
    let content_budget = width.saturating_sub(overhead);
    let truncated = truncate(content, content_budget);

    // Content text.
    let content_style = Style::default().fg(Color::White);
    spans.push(Span::styled(truncated, content_style));

    // Status badge.
    spans.push(Span::styled(
        format!(" [{badge_text}]"),
        Style::default().fg(badge_color),
    ));

    // Destination badge.
    spans.push(Span::styled(
        format!(" {dest_text}"),
        Style::default().fg(Color::DarkGray),
    ));
}

/// Append spans for an answer row: icon and content.
fn render_answer_spans(spans: &mut Vec<Span<'static>>, content: &str, width: usize) {
    // Answer icon.
    spans.push(Span::styled(
        format!("{ANSWER_ICON} "),
        Style::default().fg(Color::Green),
    ));

    // Content budget: width minus icon ("A ") and padding.
    let content_budget = width.saturating_sub(2);
    let truncated = truncate(content, content_budget);

    spans.push(Span::styled(truncated, Style::default().fg(Color::White)));
}

/// Status badge text and color for a question status.
fn status_badge(status: QuestionStatus) -> (&'static str, Color) {
    match status {
        QuestionStatus::Pending => ("Pending", Color::Yellow),
        QuestionStatus::Claimed => ("Claimed", Color::Cyan),
        QuestionStatus::PendingApproval => ("PendingApproval", Color::Magenta),
        QuestionStatus::Answered => ("Answered", Color::Green),
        QuestionStatus::TimedOut => ("TimedOut", Color::DarkGray),
        QuestionStatus::Rejected => ("Rejected", Color::Red),
    }
}

/// Destination badge string for display.
fn destination_badge(dest: QuestionDestination) -> &'static str {
    match dest {
        QuestionDestination::User => "(\u{2192}user)", // (→user)
        QuestionDestination::Llm => "(\u{2192}llm)",   // (→llm)
        QuestionDestination::Auto => "(\u{2192}auto)", // (→auto)
    }
}

/// Sort key for question status: open statuses first (0), answered (1), terminal (2).
fn status_sort_key(node: &Node) -> u8 {
    match node {
        Node::Question { status, .. } => match status {
            QuestionStatus::Pending | QuestionStatus::Claimed | QuestionStatus::PendingApproval => {
                0
            }
            QuestionStatus::Answered => 1,
            QuestionStatus::TimedOut | QuestionStatus::Rejected => 2,
        },
        _ => 3,
    }
}
