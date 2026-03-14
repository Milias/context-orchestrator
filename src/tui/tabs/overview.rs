//! Overview tab: unified dashboard combining agents, work, and stats.
//!
//! Stacks vertically: agent card, running tasks, work tree (compact),
//! then a horizontal split of recent completions and stats at the bottom.

use crate::graph::ConversationGraph;
use crate::tui::TuiState;

use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Paragraph};

use super::{agents, work};

/// Maximum number of work tree lines shown in the overview.
const WORK_TREE_MAX_LINES: u16 = 8;

/// Render the Overview tab: agent card + running tasks + work tree + bottom split.
pub fn render(frame: &mut Frame, area: Rect, graph: &ConversationGraph, tui_state: &mut TuiState) {
    let running_h = agents::running_tasks_height(graph);
    let work_h = work_tree_height(graph);

    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(agents::agent_card_height(tui_state)),
            Constraint::Length(running_h),
            Constraint::Length(work_h),
            Constraint::Min(3),
        ])
        .split(area);

    agents::render_agent_card(frame, vertical[0], tui_state);
    agents::render_running_tasks(frame, vertical[1], graph, tui_state);
    render_work_section(frame, vertical[2], graph, tui_state);

    let bottom = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(65), Constraint::Percentage(35)])
        .split(vertical[3]);

    agents::render_recent_completions(frame, bottom[0], graph, tui_state);
    crate::tui::widgets::stats_panel::render(frame, bottom[1], graph, tui_state);
}

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
