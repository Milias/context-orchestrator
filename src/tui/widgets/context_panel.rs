use crate::graph::{ConversationGraph, EdgeKind, Node, Role};
use crate::tui::{ContextTab, FocusPanel, TuiState};
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph, Tabs};

pub fn render(frame: &mut Frame, area: Rect, graph: &ConversationGraph, tui_state: &TuiState) {
    let focused = tui_state.focus == FocusPanel::ContextPanel;
    let border_color = if focused {
        Color::Yellow
    } else {
        Color::DarkGray
    };

    let outer_block = Block::default()
        .title(" Context ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color));
    let inner = outer_block.inner(area);
    frame.render_widget(outer_block, area);

    if inner.width < 6 || inner.height < 3 {
        return;
    }

    // Tab bar takes 1 row
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(1)])
        .split(inner);

    let tab_area = layout[0];
    let content_area = layout[1];

    // Render tabs
    let tab_titles: Vec<Line> = ContextTab::all()
        .iter()
        .map(|t| Line::from(t.label()))
        .collect();
    let tabs = Tabs::new(tab_titles)
        .select(tui_state.context_tab.index())
        .highlight_style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )
        .divider("│");
    frame.render_widget(tabs, tab_area);

    // Split content: 75% left (tab content), 25% right (message minimap)
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(75), Constraint::Percentage(25)])
        .split(content_area);

    render_tab_content(frame, columns[0], graph, tui_state);
    render_minimap(frame, columns[1], graph);
}

fn render_tab_content(
    frame: &mut Frame,
    area: Rect,
    graph: &ConversationGraph,
    tui_state: &TuiState,
) {
    match tui_state.context_tab {
        ContextTab::Outline => render_outline(frame, area, graph),
        ContextTab::Files => render_node_list(frame, area, graph, tui_state, is_git_file),
        ContextTab::Tools => render_node_list(frame, area, graph, tui_state, is_tool),
        ContextTab::Tasks => super::task_list::render(frame, area, graph, tui_state),
        ContextTab::Work => render_node_list(frame, area, graph, tui_state, is_work_item),
    }
}

fn render_outline(frame: &mut Frame, area: Rect, graph: &ConversationGraph) {
    let branch_names = graph.branch_names();
    let history = graph
        .get_branch_history(graph.active_branch())
        .unwrap_or_default();

    let msg_count = history
        .iter()
        .filter(|n| matches!(n, Node::Message { .. }))
        .count();

    let total_in: u32 = history.iter().filter_map(|n| n.input_tokens()).sum();
    let total_out: u32 = history.iter().filter_map(|n| n.output_tokens()).sum();

    let mut lines = Vec::new();
    for name in &branch_names {
        let marker = if *name == graph.active_branch() {
            "*"
        } else {
            " "
        };
        lines.push(Line::from(Span::styled(
            format!("{marker} {name}"),
            Style::default().fg(Color::Cyan),
        )));
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        format!("{msg_count} messages"),
        Style::default().fg(Color::White),
    )));

    if total_in > 0 || total_out > 0 {
        lines.push(Line::from(Span::styled(
            format!("{total_in} in / {total_out} out tokens"),
            Style::default().fg(Color::DarkGray),
        )));
    }

    let paragraph = Paragraph::new(lines);
    frame.render_widget(paragraph, area);
}

fn render_node_list(
    frame: &mut Frame,
    area: Rect,
    graph: &ConversationGraph,
    tui_state: &TuiState,
    filter: fn(&Node) -> bool,
) {
    let mut nodes = graph.nodes_by(filter);
    nodes.sort_by_key(|n| n.content().to_string());

    if nodes.is_empty() {
        let paragraph =
            Paragraph::new(Span::styled("(none)", Style::default().fg(Color::DarkGray)));
        frame.render_widget(paragraph, area);
        return;
    }

    let offset = tui_state
        .context_list_offset
        .min(nodes.len().saturating_sub(1));

    let items: Vec<ListItem> = nodes
        .iter()
        .skip(offset)
        .map(|node| {
            let line = format_node_line(node);
            ListItem::new(line)
        })
        .collect();

    let list = List::new(items);
    frame.render_widget(list, area);
}

fn format_node_line(node: &Node) -> Line<'static> {
    match node {
        Node::GitFile { path, status, .. } => {
            let marker = match status {
                crate::graph::GitFileStatus::Modified => "[M]",
                crate::graph::GitFileStatus::Staged => "[S]",
                crate::graph::GitFileStatus::Untracked => "[?]",
                crate::graph::GitFileStatus::Tracked => "[ ]",
            };
            Line::from(vec![
                Span::styled(format!("{marker} "), Style::default().fg(Color::Yellow)),
                Span::styled(path.clone(), Style::default().fg(Color::Blue)),
            ])
        }
        Node::Tool {
            name, description, ..
        } => Line::from(vec![
            Span::styled(
                name.clone(),
                Style::default()
                    .fg(Color::Magenta)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("  {description}"),
                Style::default().fg(Color::DarkGray),
            ),
        ]),
        Node::WorkItem { title, status, .. } => {
            let (marker, color) = match status {
                crate::graph::WorkItemStatus::Todo => ("[]", Color::Yellow),
                crate::graph::WorkItemStatus::Active => ("[*]", Color::Cyan),
                crate::graph::WorkItemStatus::Done => ("[v]", Color::Green),
            };
            Line::from(vec![
                Span::styled(format!("{marker} "), Style::default().fg(color)),
                Span::styled(title.clone(), Style::default().fg(Color::White)),
            ])
        }
        _ => Line::from(node.content().to_string()),
    }
}

fn render_minimap(frame: &mut Frame, area: Rect, graph: &ConversationGraph) {
    let history = graph
        .get_branch_history(graph.active_branch())
        .unwrap_or_default();

    let block = Block::default()
        .title("Messages")
        .borders(Borders::LEFT)
        .border_style(Style::default().fg(Color::DarkGray));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.width < 3 || inner.height == 0 {
        return;
    }

    let width = inner.width as usize;
    let max_len = width.saturating_sub(3);
    let mut items: Vec<ListItem> = Vec::new();

    for node in &history {
        let (prefix, color) = match node {
            Node::Message { role, .. } => match role {
                Role::User => ("U", Color::Cyan),
                Role::Assistant => ("A", Color::Green),
                Role::System => ("S", Color::DarkGray),
            },
            Node::SystemDirective { .. } => ("S", Color::DarkGray),
            _ => continue,
        };
        let content = node.content();
        let truncated: String = content.chars().take(max_len).collect();
        items.push(minimap_item(prefix, color, &truncated));

        // Inject tool calls/results after assistant messages
        if matches!(
            node,
            Node::Message {
                role: Role::Assistant,
                ..
            }
        ) {
            for tc_id in &graph.sources_by_edge(node.id(), EdgeKind::Invoked) {
                if let Some(tc_node) = graph.node(*tc_id) {
                    let name = tc_node.content();
                    let tc_text: String = name.chars().take(max_len).collect();
                    items.push(minimap_item("T", Color::Magenta, &tc_text));

                    for r_id in &graph.sources_by_edge(*tc_id, EdgeKind::Produced) {
                        if let Some(r_node) = graph.node(*r_id) {
                            let is_err = matches!(r_node, Node::ToolResult { is_error: true, .. });
                            let r_color = if is_err { Color::Red } else { Color::DarkGray };
                            let r_text: String = r_node.content().chars().take(max_len).collect();
                            items.push(minimap_item("R", r_color, &r_text));
                        }
                    }
                }
            }
        }
    }

    // Show only the last items that fit in the viewport
    let visible = inner.height as usize;
    let skip = items.len().saturating_sub(visible);
    let list = List::new(items.into_iter().skip(skip).collect::<Vec<_>>());
    frame.render_widget(list, inner);
}

fn minimap_item<'a>(prefix: &str, color: Color, text: &str) -> ListItem<'a> {
    ListItem::new(Line::from(vec![
        Span::styled(
            format!("{prefix} "),
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        ),
        Span::styled(text.to_string(), Style::default().fg(Color::White)),
    ]))
}

fn is_git_file(node: &Node) -> bool {
    matches!(node, Node::GitFile { .. })
}

fn is_tool(node: &Node) -> bool {
    matches!(node, Node::Tool { .. })
}

fn is_work_item(node: &Node) -> bool {
    matches!(node, Node::WorkItem { .. })
}
