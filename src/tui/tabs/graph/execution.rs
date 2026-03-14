//! Execution section tree for the Graph tab.
//!
//! Renders assistant messages as tree roots, with tool calls as children
//! (via `Invoked` edges) and tool results as grandchildren (via `Produced`
//! edges). Provides navigable tree with selection, collapse/expand,
//! and tree-command-style connectors.

use std::cmp::Reverse;

use chrono::Utc;
use ratatui::{
    prelude::*,
    widgets::{Block, Borders, Paragraph},
};
use uuid::Uuid;

use crate::graph::{tool_types::ToolCallStatus, ConversationGraph, EdgeKind, Node, Role};
use crate::tui::state::GraphSection;
use crate::tui::tabs::{explorer::ExplorerState, graph::tree_lines::TreePrefix};
use crate::tui::widgets::tool_status::{
    elapsed, finished, format_duration, tool_call_status_icon, truncate, TaskDuration,
};
use crate::tui::TuiState;

/// Flattened execution tree node carrying its tree prefix and display data.
struct FlatItem {
    /// Graph node UUID for selection mapping and detail panel.
    id: Uuid,
    /// Pre-rendered tree connector prefix (e.g. `"├── "`).
    prefix: String,
    /// The line spans to render (pre-built for each node type).
    spans: Vec<Span<'static>>,
}

/// Graceful fallback `FlatItem` for unexpected node types (avoids panicking).
fn unexpected_item(node: &Node, prefix: &TreePrefix, is_last: bool) -> FlatItem {
    FlatItem {
        id: node.id(),
        prefix: prefix.render(is_last),
        spans: vec![Span::styled(
            "(unexpected node)",
            Style::default().fg(Color::DarkGray),
        )],
    }
}

/// Render the Execution section tree within the Graph tab.
///
/// Assistant messages are roots, `ToolCall` nodes are children (via `Invoked`
/// edge), `ToolResult` nodes are grandchildren (via `Produced` edge).
/// Returns the UUID of the currently selected node (if any) so the caller
/// can pass it to the detail panel.
pub fn render(
    frame: &mut Frame,
    tree_area: Rect,
    graph: &ConversationGraph,
    tui_state: &mut TuiState,
) -> Option<Uuid> {
    let block = Block::default().title("Execution").borders(Borders::ALL);
    let inner = block.inner(tree_area);
    frame.render_widget(block, tree_area);

    if inner.height == 0 || inner.width < 10 {
        return None;
    }

    let explorer = tui_state
        .explorer
        .get_mut(&GraphSection::Execution)
        .expect("Execution explorer state must exist");

    let width = inner.width as usize;
    let flat_items = build_flat_tree(graph, explorer, width);

    if flat_items.is_empty() {
        let empty = Paragraph::new(Span::styled(
            "(no assistant messages)",
            Style::default().fg(Color::DarkGray),
        ));
        frame.render_widget(empty, inner);
        explorer.visible_count = 0;
        return None;
    }

    explorer.visible_count = flat_items.len();
    if explorer.selected >= flat_items.len() {
        explorer.selected = flat_items.len().saturating_sub(1);
    }

    let selected_id = flat_items.get(explorer.selected).map(|item| item.id);
    let selected_idx = explorer.selected;
    let max_lines = inner.height as usize;

    let lines: Vec<Line<'_>> = flat_items
        .iter()
        .take(max_lines)
        .enumerate()
        .map(|(i, item)| render_flat_item(item, i, selected_idx))
        .collect();

    frame.render_widget(Paragraph::new(Text::from(lines)), inner);

    selected_id
}

/// Build a flattened execution tree in display order.
///
/// Collects all assistant messages, sorts newest-first, then for each
/// walks Invoked edges to tool calls and Produced edges to tool results.
fn build_flat_tree(
    graph: &ConversationGraph,
    explorer: &ExplorerState,
    width: usize,
) -> Vec<FlatItem> {
    let mut roots: Vec<&Node> = graph
        .nodes_by(|n| {
            matches!(
                n,
                Node::Message {
                    role: Role::Assistant,
                    ..
                }
            )
        })
        .into_iter()
        .collect();

    // Newest first so most recent activity is at the top.
    roots.sort_by_key(|n| Reverse(n.created_at()));

    let mut flat = Vec::new();
    let total_roots = roots.len();
    let now = Utc::now();

    for (i, root) in roots.iter().enumerate() {
        let is_last_root = i + 1 == total_roots;
        let root_id = root.id();

        // Tool calls invoked by this assistant message.
        let tool_call_ids = graph.sources_by_edge(root_id, EdgeKind::Invoked);
        let has_children = !tool_call_ids.is_empty();
        let is_collapsed = explorer.is_collapsed(&root_id);

        flat.push(build_assistant_item(
            root,
            &TreePrefix::new(),
            is_last_root,
            has_children,
            is_collapsed,
            width,
        ));

        if has_children && !is_collapsed {
            let child_prefix = TreePrefix::new().child(is_last_root);
            let total_tc = tool_call_ids.len();

            for (j, tc_id) in tool_call_ids.iter().enumerate() {
                let is_last_tc = j + 1 == total_tc;

                let Some(tc_node) = graph.node(*tc_id) else {
                    continue;
                };

                // Tool results produced by this tool call.
                let result_ids = graph.sources_by_edge(*tc_id, EdgeKind::Produced);
                let tc_has_children = !result_ids.is_empty();
                let tc_collapsed = explorer.is_collapsed(tc_id);

                flat.push(build_tool_call_item(
                    tc_node,
                    &child_prefix,
                    is_last_tc,
                    tc_has_children,
                    tc_collapsed,
                    now,
                    width,
                ));

                if tc_has_children && !tc_collapsed {
                    let grandchild_prefix = child_prefix.child(is_last_tc);
                    let result_count = result_ids.len();

                    for (k, tr_id) in result_ids.iter().enumerate() {
                        let is_last_result = k + 1 == result_count;
                        if let Some(tr_node) = graph.node(*tr_id) {
                            flat.push(build_tool_result_item(
                                tr_node,
                                &grandchild_prefix,
                                is_last_result,
                                width,
                            ));
                        }
                    }
                }
            }
        }
    }

    flat
}

/// Build a flat item for an assistant message root node.
fn build_assistant_item(
    node: &Node,
    prefix: &TreePrefix,
    is_last: bool,
    has_children: bool,
    is_collapsed: bool,
    width: usize,
) -> FlatItem {
    let Node::Message {
        id,
        content,
        created_at,
        ..
    } = node
    else {
        return unexpected_item(node, prefix, is_last);
    };

    let prefix_str = prefix.render(is_last);
    let timestamp = created_at.format("[%H:%M:%S]").to_string();

    // Budget: prefix + collapse + "A " + content + " " + timestamp.
    let collapse = collapse_indicator(has_children, is_collapsed);
    let fixed_width = prefix_str.chars().count()
        + collapse.chars().count()
        + 2  // "A "
        + 1  // space before timestamp
        + timestamp.chars().count();
    let content_budget = width.saturating_sub(fixed_width);
    let first_line = content.lines().next().unwrap_or("");
    let display_content = truncate(first_line, content_budget);

    let spans = vec![
        Span::styled(collapse.to_string(), Style::default().fg(Color::DarkGray)),
        Span::styled("A ", Style::default().fg(Color::Green).bold()),
        Span::styled(display_content, Style::default().fg(Color::White)),
        Span::styled(
            format!(" {timestamp}"),
            Style::default().fg(Color::DarkGray),
        ),
    ];

    FlatItem {
        id: *id,
        prefix: prefix_str,
        spans,
    }
}

/// Build a flat item for a tool call child node.
fn build_tool_call_item(
    node: &Node,
    prefix: &TreePrefix,
    is_last: bool,
    has_children: bool,
    is_collapsed: bool,
    now: chrono::DateTime<Utc>,
    width: usize,
) -> FlatItem {
    let Node::ToolCall {
        id,
        arguments,
        status,
        created_at,
        completed_at,
        ..
    } = node
    else {
        return unexpected_item(node, prefix, is_last);
    };

    let prefix_str = prefix.render(is_last);
    let (icon, icon_color) = tool_call_status_icon(status);

    let duration = match status {
        ToolCallStatus::Pending => TaskDuration::Pending,
        ToolCallStatus::Running => elapsed(now, *created_at),
        _ => match completed_at {
            Some(end) => finished(*end, *created_at),
            None => finished(now, *created_at),
        },
    };
    let dur_str = format_duration(&duration);
    let dur_color = if matches!(status, ToolCallStatus::Pending | ToolCallStatus::Running) {
        Color::Yellow
    } else {
        Color::DarkGray
    };

    let (tool_name, args_summary) = arguments.display_parts();
    let collapse = collapse_indicator(has_children, is_collapsed);

    // Budget: prefix + collapse + icon + " " + name + " " + args + " [" + dur + "]".
    let fixed_width = prefix_str.chars().count()
        + collapse.chars().count()
        + icon.chars().count()
        + 1  // space after icon
        + tool_name.chars().count()
        + 2  // " [" before duration
        + dur_str.chars().count()
        + 1; // "]"
    let args_budget = width.saturating_sub(fixed_width + 1); // +1 for space before args
    let args_display = if args_summary.is_empty() {
        String::new()
    } else {
        format!(" {}", truncate(&args_summary, args_budget))
    };

    let spans = vec![
        Span::styled(collapse.to_string(), Style::default().fg(Color::DarkGray)),
        Span::styled(format!("{icon} "), Style::default().fg(icon_color)),
        Span::styled(
            tool_name.to_string(),
            Style::default().fg(Color::Magenta).bold(),
        ),
        Span::styled(args_display, Style::default().fg(Color::White)),
        Span::styled(format!(" [{dur_str}]"), Style::default().fg(dur_color)),
    ];

    FlatItem {
        id: *id,
        prefix: prefix_str,
        spans,
    }
}

/// Build a flat item for a tool result grandchild node.
fn build_tool_result_item(
    node: &Node,
    prefix: &TreePrefix,
    is_last: bool,
    width: usize,
) -> FlatItem {
    let Node::ToolResult {
        id,
        content,
        is_error,
        ..
    } = node
    else {
        return unexpected_item(node, prefix, is_last);
    };

    let prefix_str = prefix.render(is_last);
    let result_label = "Result: ";
    let color = if *is_error {
        Color::Red
    } else {
        Color::DarkGray
    };

    // Budget: prefix + "-> " + "Result: " + content.
    let fixed_width = prefix_str.chars().count() + 3 + result_label.chars().count();
    let content_budget = width.saturating_sub(fixed_width);
    let first_line = content.text_content().lines().next().unwrap_or("");
    let display_content = truncate(first_line, content_budget);

    let spans = vec![
        Span::styled("\u{2192} ", Style::default().fg(Color::DarkGray)), // →
        Span::styled(result_label, Style::default().fg(color)),
        Span::styled(display_content, Style::default().fg(color)),
    ];

    FlatItem {
        id: *id,
        prefix: prefix_str,
        spans,
    }
}

/// Render a single flat item as a styled `Line`.
fn render_flat_item(item: &FlatItem, line_idx: usize, selected_idx: usize) -> Line<'static> {
    let is_selected = line_idx == selected_idx;
    let dim = Style::default().fg(Color::DarkGray);

    let mut spans = Vec::new();
    spans.push(Span::styled(item.prefix.clone(), dim));
    spans.extend(item.spans.clone());
    // Apply background highlight to all spans on the selected line.
    if is_selected {
        let bg = Color::Rgb(40, 40, 60);
        for span in &mut spans {
            span.style = span.style.bg(bg);
        }
    }

    Line::from(spans)
}

/// Collapse/expand indicator for nodes with children.
fn collapse_indicator(has_children: bool, is_collapsed: bool) -> &'static str {
    if !has_children {
        return "";
    }
    if is_collapsed {
        "\u{25b6} " // ▶
    } else {
        "\u{25bc} " // ▼
    }
}
