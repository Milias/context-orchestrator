//! Work tree for the Graph tab: plans and tasks with tree connectors.
//!
//! Renders work items as a navigable tree with `tree`-command-style
//! connectors (via [`TreePrefix`]), expand/collapse support, selection
//! highlighting, and inline edge badges (claimed-by agent, open questions).

use uuid::Uuid;

use crate::graph::node::{WorkItemKind, WorkItemStatus};
use crate::graph::{ConversationGraph, EdgeKind, Node};
use crate::tui::state::GraphSection;
use crate::tui::tabs::explorer::ExplorerState;
use crate::tui::tabs::graph::tree_lines::TreePrefix;
use crate::tui::widgets::tool_status::truncate;
use crate::tui::TuiState;

use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Paragraph};

/// A flattened work-tree node carrying its tree prefix and display data.
struct FlatItem {
    /// Graph node UUID for selection mapping and detail panel.
    id: Uuid,
    /// Pre-rendered tree connector prefix (e.g. `"├── "`).
    prefix: String,
    /// Work item title.
    title: String,
    /// Plan or Task.
    kind: WorkItemKind,
    /// Current status (Todo, Active, Done).
    status: WorkItemStatus,
    /// Dependency titles for inline annotation.
    deps: Vec<String>,
    /// Whether this node has children (for collapse indicator).
    has_children: bool,
    /// Whether this node is collapsed in the explorer state.
    is_collapsed: bool,
    /// Short agent UUID suffix if claimed, e.g. `"7b1c"`.
    claimed_by: Option<String>,
    /// Number of open questions about this work item.
    open_questions: usize,
}

/// Build and render the work tree with tree-command-style connectors.
///
/// Returns the UUID of the currently selected node (if any) so the
/// caller can pass it to the detail panel.
pub fn render(
    frame: &mut Frame,
    tree_area: Rect,
    graph: &ConversationGraph,
    tui_state: &mut TuiState,
) -> Option<Uuid> {
    let block = Block::default().title("Work").borders(Borders::ALL);
    let inner = block.inner(tree_area);
    frame.render_widget(block, tree_area);

    if inner.height == 0 || inner.width < 10 {
        return None;
    }

    let explorer = tui_state
        .explorer
        .get_mut(&GraphSection::Work)
        .expect("Work explorer state must exist");

    let flat_items = build_flat_tree(graph, explorer);

    if flat_items.is_empty() {
        let empty = Paragraph::new(Span::styled(
            "(no plans or tasks)",
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

/// Build a flattened list of work items in tree-walk order.
///
/// Root nodes are work items with no `SubtaskOf` parent. Plans sort
/// before tasks, then alphabetically by title. Children are expanded
/// or collapsed based on the explorer state.
fn build_flat_tree(graph: &ConversationGraph, explorer: &ExplorerState) -> Vec<FlatItem> {
    let work_items: Vec<&Node> = graph.nodes_by(|n| matches!(n, Node::WorkItem { .. }));

    // Collect root items (no SubtaskOf parent).
    let mut roots: Vec<&Node> = work_items
        .iter()
        .filter(|n| graph.parent_of(n.id()).is_none())
        .copied()
        .collect();

    // Sort: plans first, then alphabetically.
    roots.sort_by(|a, b| {
        kind_order(a)
            .cmp(&kind_order(b))
            .then_with(|| a.content().cmp(b.content()))
    });

    let mut flat = Vec::new();
    let total_roots = roots.len();
    for (i, root) in roots.iter().enumerate() {
        let is_last = i + 1 == total_roots;
        flatten_node(
            graph,
            explorer,
            root,
            &TreePrefix::new(),
            is_last,
            &mut flat,
        );
    }
    flat
}

/// Recursively flatten a node and its visible children into the flat list.
fn flatten_node(
    graph: &ConversationGraph,
    explorer: &ExplorerState,
    node: &Node,
    prefix: &TreePrefix,
    is_last_sibling: bool,
    out: &mut Vec<FlatItem>,
) {
    let id = node.id();
    let child_ids = graph.children_of(id);

    let mut children: Vec<&Node> = child_ids
        .iter()
        .filter_map(|cid| graph.node(*cid))
        .collect();
    children.sort_by(|a, b| {
        kind_order(a)
            .cmp(&kind_order(b))
            .then_with(|| a.content().cmp(b.content()))
    });

    let has_children = !children.is_empty();
    let is_collapsed = explorer.is_collapsed(&id);

    // Extract work item fields.
    let (title, kind, status) = match node {
        Node::WorkItem {
            title,
            kind,
            status,
            ..
        } => (title.clone(), *kind, *status),
        _ => return,
    };

    // Dependency titles.
    let deps: Vec<String> = graph
        .dependencies_of(id)
        .iter()
        .filter_map(|dep_id| graph.node(*dep_id).map(|n| n.content().to_string()))
        .collect();

    // Claimed-by agent badge: look for outgoing ClaimedBy edge.
    let claimed_by = graph
        .targets_by_edge(id, EdgeKind::ClaimedBy)
        .first()
        .map(short_uuid);

    // Open questions count: incoming Asks edges pointing at this node (via About).
    let open_questions = count_open_questions(graph, id);

    out.push(FlatItem {
        id,
        prefix: prefix.render(is_last_sibling),
        title,
        kind,
        status,
        deps,
        has_children,
        is_collapsed,
        claimed_by,
        open_questions,
    });

    // Recurse into visible children.
    if has_children && !is_collapsed {
        let child_prefix = prefix.child(is_last_sibling);
        let total = children.len();
        for (i, child) in children.iter().enumerate() {
            let child_is_last = i + 1 == total;
            flatten_node(graph, explorer, child, &child_prefix, child_is_last, out);
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
    let (icon, icon_color) = status_icon(item.status);

    // Collapse/expand indicator for items with children.
    let collapse_indicator = if item.has_children {
        if item.is_collapsed {
            "\u{25b6} "
        } else {
            "\u{25bc} "
        } // ▶ or ▼
    } else {
        ""
    };

    let kind_prefix = match item.kind {
        WorkItemKind::Plan => "Plan: ",
        WorkItemKind::Task => "",
    };

    let prefix_width = item.prefix.chars().count()
        + collapse_indicator.chars().count()
        + icon.chars().count()
        + 1 // space after icon
        + kind_prefix.chars().count();
    let title_budget = width.saturating_sub(prefix_width);
    let title = truncate(&item.title, title_budget);

    let base_color = match item.kind {
        WorkItemKind::Plan => Color::Yellow,
        WorkItemKind::Task => Color::White,
    };
    let title_style = if is_selected {
        Style::default()
            .fg(base_color)
            .bg(Color::Rgb(40, 40, 60))
            .add_modifier(Modifier::BOLD)
    } else if matches!(item.kind, WorkItemKind::Plan) {
        Style::default().fg(base_color).bold()
    } else {
        Style::default().fg(base_color)
    };

    let dim = Style::default().fg(Color::DarkGray);
    let mut spans = Vec::new();

    spans.push(Span::styled(item.prefix.clone(), dim));
    spans.push(Span::styled(collapse_indicator, dim));
    spans.push(Span::styled(
        format!("{icon} "),
        Style::default().fg(icon_color),
    ));
    spans.push(Span::styled(kind_prefix, dim));
    spans.push(Span::styled(title, title_style));

    // Inline badges.
    append_badges(&mut spans, item);

    // Apply background highlight to all spans on the selected line.
    if is_selected {
        let bg = Color::Rgb(40, 40, 60);
        for span in &mut spans {
            span.style = span.style.bg(bg);
        }
    }

    Line::from(spans)
}

/// Append inline edge badges to the span list.
///
/// Shows claimed-by agent short UUID and open question count
/// when present on the work item.
fn append_badges(spans: &mut Vec<Span<'static>>, item: &FlatItem) {
    let badge_style = Style::default().fg(Color::Rgb(100, 100, 140));

    if let Some(agent_short) = &item.claimed_by {
        spans.push(Span::styled(
            format!(" \u{2190} agent:{agent_short}"), // ← agent:XXXX
            badge_style,
        ));
    }

    if item.open_questions > 0 {
        spans.push(Span::styled(
            format!(" ?{}", item.open_questions),
            Style::default().fg(Color::Magenta),
        ));
    }

    if !item.deps.is_empty() {
        let dep_names: String = item
            .deps
            .iter()
            .map(|d| truncate(d, 20))
            .collect::<Vec<_>>()
            .join(", ");
        spans.push(Span::styled(
            "  (needs: ",
            Style::default().fg(Color::DarkGray),
        ));
        spans.push(Span::styled(dep_names, Style::default().fg(Color::Magenta)));
        spans.push(Span::styled(")", Style::default().fg(Color::DarkGray)));
    }
}

/// Status icon and color for a work item status.
fn status_icon(status: WorkItemStatus) -> (&'static str, Color) {
    match status {
        WorkItemStatus::Todo => ("[ ]", Color::DarkGray),
        WorkItemStatus::Active => ("[*]", Color::Yellow),
        WorkItemStatus::Done => ("[v]", Color::Green),
    }
}

/// Sort key: plans before tasks.
fn kind_order(node: &Node) -> u8 {
    match node {
        Node::WorkItem {
            kind: WorkItemKind::Plan,
            ..
        } => 0,
        _ => 1,
    }
}

/// Short UUID suffix (last 4 hex chars) for compact display.
fn short_uuid(id: &Uuid) -> String {
    let s = id.to_string();
    s[s.len().saturating_sub(4)..].to_string()
}

/// Count open questions about a work item.
///
/// Looks for `Question` nodes that have an `About` edge pointing to
/// this work item and are not yet `Answered` or `TimedOut`.
fn count_open_questions(graph: &ConversationGraph, work_item_id: Uuid) -> usize {
    graph
        .sources_by_edge(work_item_id, EdgeKind::About)
        .iter()
        .filter(|&qid| {
            matches!(
                graph.node(*qid),
                Some(Node::Question { status, .. })
                    if !matches!(
                        status,
                        crate::graph::node::QuestionStatus::Answered
                            | crate::graph::node::QuestionStatus::TimedOut
                    )
            )
        })
        .count()
}
