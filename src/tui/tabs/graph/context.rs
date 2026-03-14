//! Context provenance tree for the Graph tab.
//!
//! Renders `ContextBuildingRequest` (CBR) nodes as tree roots, with selected
//! nodes grouped by tier (Essential/Important/Supplementary) as children.
//! Uses [`TreePrefix`] for tree-command-style connectors.

use std::cmp::Reverse;

use uuid::Uuid;

use crate::graph::node::{ContextBuildStatus, ContextPolicyKind};
use crate::graph::{ConversationGraph, EdgeKind, Node};
use crate::tui::state::{ExplorerFocus, GraphSection};
use crate::tui::tabs::explorer::ExplorerState;
use crate::tui::tabs::graph::tree_lines::TreePrefix;
use crate::tui::ui::format_token_count;
use crate::tui::widgets::tool_status::truncate;
use crate::tui::TuiState;

use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Paragraph};

/// Display label for the Essential tier group header.
const TIER_ESSENTIAL: &str = "Essential";
/// Display label for the Important tier group header.
const TIER_IMPORTANT: &str = "Important";
/// Display label for the Supplementary tier group header.
const TIER_SUPPLEMENTARY: &str = "Supplementary";

/// A flattened tree item carrying prefix, display data, and optional UUID.
///
/// Tier group headers have `id: None` (they are virtual grouping labels,
/// not real graph nodes). CBR roots and selected leaf nodes carry their
/// graph node UUID for selection and detail panel navigation.
struct FlatItem {
    /// Graph node UUID, or `None` for tier group headers.
    id: Option<Uuid>,
    /// Pre-rendered tree connector prefix (e.g. `"├── "`).
    prefix: String,
    /// Display text for this line.
    label: String,
    /// Visual kind determines styling.
    kind: ItemKind,
    /// Whether this node has children (for collapse indicator).
    has_children: bool,
    /// Whether this node is collapsed in the explorer state.
    is_collapsed: bool,
}

/// Visual kind of a flat item, used for styling.
enum ItemKind {
    /// CBR root node: shows policy, agent, status, tokens.
    CbrRoot,
    /// Tier group header (Essential/Important/Supplementary).
    TierGroup,
    /// Selected leaf node with its type badge.
    SelectedNode,
}

/// Render the Context section tree within the Graph tab.
///
/// `ContextBuildingRequest` nodes are roots, selected nodes are children
/// grouped by tier (Essential/Important/Supplementary). Returns the UUID
/// of the currently selected node (if any) for the detail panel.
pub fn render(
    frame: &mut Frame,
    tree_area: Rect,
    graph: &ConversationGraph,
    tui_state: &mut TuiState,
) -> Option<Uuid> {
    let focused = tui_state
        .explorer
        .get(&GraphSection::Context)
        .is_none_or(|e| e.focus == ExplorerFocus::Tree);
    let border_style = if focused {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    };
    let block = Block::default()
        .title("Context")
        .borders(Borders::ALL)
        .border_style(border_style);
    let inner = block.inner(tree_area);
    frame.render_widget(block, tree_area);

    if inner.height == 0 || inner.width < 10 {
        return None;
    }

    let explorer = tui_state
        .explorer
        .get_mut(&GraphSection::Context)
        .expect("Context explorer state must exist");

    let flat_items = build_flat_tree(graph, explorer);

    if flat_items.is_empty() {
        let empty = Paragraph::new(Span::styled(
            "(no context builds)",
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

    let selected_id = flat_items.get(explorer.selected).and_then(|item| item.id);
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

/// Build a flattened list of context items in tree-walk order.
///
/// CBR nodes are roots (sorted newest-first by `created_at`). Under each
/// CBR, selected nodes are grouped into tier headers with leaf children.
fn build_flat_tree(graph: &ConversationGraph, explorer: &ExplorerState) -> Vec<FlatItem> {
    let mut cbr_nodes: Vec<&Node> =
        graph.nodes_by(|n| matches!(n, Node::ContextBuildingRequest { .. }));

    // Sort newest first by created_at.
    cbr_nodes.sort_by_key(|n| Reverse(n.created_at()));

    let mut flat = Vec::new();
    let total_roots = cbr_nodes.len();

    for (i, cbr) in cbr_nodes.iter().enumerate() {
        let is_last_root = i + 1 == total_roots;
        flatten_cbr(graph, explorer, cbr, is_last_root, &mut flat);
    }

    flat
}

/// Flatten a single CBR node and its tier-grouped selected nodes.
fn flatten_cbr(
    graph: &ConversationGraph,
    explorer: &ExplorerState,
    cbr: &Node,
    is_last_root: bool,
    out: &mut Vec<FlatItem>,
) {
    let cbr_id = cbr.id();
    let label = format_cbr_label(cbr);
    let selected_ids = graph.targets_by_edge(cbr_id, EdgeKind::SelectedFor);
    let has_children = !selected_ids.is_empty();
    let is_collapsed = explorer.is_collapsed(&cbr_id);

    let root_prefix = TreePrefix::new();
    out.push(FlatItem {
        id: Some(cbr_id),
        prefix: root_prefix.render(is_last_root),
        label,
        kind: ItemKind::CbrRoot,
        has_children,
        is_collapsed,
    });

    if !has_children || is_collapsed {
        return;
    }

    // Collect and classify selected nodes into tiers.
    let tiers = classify_tiers(graph, &selected_ids);
    let child_prefix = root_prefix.child(is_last_root);
    let non_empty_count = tiers.iter().filter(|(_, nodes)| !nodes.is_empty()).count();
    let mut rendered = 0usize;

    for (tier_label, nodes) in &tiers {
        if nodes.is_empty() {
            continue;
        }
        rendered += 1;
        let is_last_tier = rendered == non_empty_count;
        flatten_tier_group(&child_prefix, tier_label, nodes, is_last_tier, out);
    }
}

/// Flatten a tier group header and its selected node children.
fn flatten_tier_group(
    parent_prefix: &TreePrefix,
    tier_label: &str,
    nodes: &[&Node],
    is_last_tier: bool,
    out: &mut Vec<FlatItem>,
) {
    let group_label = format!("{tier_label} ({} nodes)", nodes.len());
    out.push(FlatItem {
        id: None,
        prefix: parent_prefix.render(is_last_tier),
        label: group_label,
        kind: ItemKind::TierGroup,
        has_children: false,
        is_collapsed: false,
    });

    let leaf_prefix = parent_prefix.child(is_last_tier);
    let total = nodes.len();
    for (i, node) in nodes.iter().enumerate() {
        let is_last_leaf = i + 1 == total;
        let node_label = format!("{}: {}", node.type_badge(), first_line(node.content()),);
        out.push(FlatItem {
            id: Some(node.id()),
            prefix: leaf_prefix.render(is_last_leaf),
            label: node_label,
            kind: ItemKind::SelectedNode,
            has_children: false,
            is_collapsed: false,
        });
    }
}

/// Classify selected nodes into (Essential, Important, Supplementary) tiers.
///
/// Grouping is by node type as a visual approximation:
/// - **Essential**: `Message` and `WorkItem` (conversation chain and task context)
/// - **Important**: `ToolCall` and `ToolResult` (execution context)
/// - **Supplementary**: everything else (`GitFile`, `BackgroundTask`, `ApiError`, etc.)
fn classify_tiers<'g>(
    graph: &'g ConversationGraph,
    selected_ids: &[Uuid],
) -> Vec<(&'static str, Vec<&'g Node>)> {
    let mut essential = Vec::new();
    let mut important = Vec::new();
    let mut supplementary = Vec::new();

    for &id in selected_ids {
        let Some(node) = graph.node(id) else {
            continue;
        };
        match node {
            Node::Message { .. } | Node::WorkItem { .. } => essential.push(node),
            Node::ToolCall { .. } | Node::ToolResult { .. } => important.push(node),
            _ => supplementary.push(node),
        }
    }

    vec![
        (TIER_ESSENTIAL, essential),
        (TIER_IMPORTANT, important),
        (TIER_SUPPLEMENTARY, supplementary),
    ]
}

/// Format the CBR root label.
///
/// Format: `CBR: {policy} (agent:{short_id}) [{status}, {token_count} tok]`
fn format_cbr_label(node: &Node) -> String {
    let Node::ContextBuildingRequest {
        policy,
        status,
        token_count,
        agent_id,
        ..
    } = node
    else {
        return String::new();
    };

    let policy_str = match policy {
        ContextPolicyKind::Conversational => "Conversational",
        ContextPolicyKind::TaskExecution => "TaskExecution",
    };
    let status_str = format_build_status(*status);
    let agent_short: String = agent_id.to_string().chars().take(8).collect();
    let tok_str = token_count.map_or_else(
        || String::from("-"),
        |t| format!("{} tok", format_token_count(u64::from(t))),
    );

    format!("CBR: {policy_str} (agent:{agent_short}) [{status_str}, {tok_str}]")
}

/// Human-readable label for a `ContextBuildStatus`.
fn format_build_status(status: ContextBuildStatus) -> &'static str {
    match status {
        ContextBuildStatus::Requested => "Requested",
        ContextBuildStatus::Building => "Building",
        ContextBuildStatus::Built => "Built",
        ContextBuildStatus::Consumed => "Consumed",
        ContextBuildStatus::FallbackUsed => "Fallback",
        ContextBuildStatus::Failed => "Failed",
    }
}

/// Extract the first line of a content string, trimmed.
fn first_line(content: &str) -> &str {
    content.lines().next().unwrap_or("").trim()
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

    // Collapse/expand indicator for CBR roots with children.
    let collapse_indicator = if item.has_children {
        if item.is_collapsed {
            "\u{25b6} "
        } else {
            "\u{25bc} "
        }
    } else {
        ""
    };

    let prefix_width = item.prefix.chars().count() + collapse_indicator.chars().count();
    let label_budget = width.saturating_sub(prefix_width);
    let label = truncate(&item.label, label_budget);

    let (label_color, label_bold) = match item.kind {
        ItemKind::CbrRoot => (Color::Yellow, true),
        ItemKind::TierGroup => (Color::DarkGray, false),
        ItemKind::SelectedNode => (Color::White, false),
    };

    let label_style = if is_selected {
        let mut s = Style::default().fg(label_color).bg(Color::Rgb(40, 40, 60));
        if label_bold {
            s = s.add_modifier(Modifier::BOLD);
        }
        s
    } else if label_bold {
        Style::default().fg(label_color).bold()
    } else {
        Style::default().fg(label_color)
    };

    let mut spans = Vec::new();

    spans.push(Span::styled(item.prefix.clone(), dim));
    spans.push(Span::styled(collapse_indicator, dim));
    spans.push(Span::styled(label, label_style));

    // Apply background highlight to all spans on the selected line.
    if is_selected {
        let bg = Color::Rgb(40, 40, 60);
        for span in &mut spans {
            span.style = span.style.bg(bg);
        }
    }

    Line::from(spans)
}
