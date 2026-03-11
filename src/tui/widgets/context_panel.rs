use crate::graph::{ConversationGraph, Node, Role};
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
        .highlight_style(Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD))
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
        ContextTab::Tasks => render_node_list(frame, area, graph, tui_state, is_background_task),
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
        let paragraph = Paragraph::new(Span::styled("(none)", Style::default().fg(Color::DarkGray)));
        frame.render_widget(paragraph, area);
        return;
    }

    let offset = tui_state.context_list_offset.min(nodes.len().saturating_sub(1));

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
            Span::styled(name.clone(), Style::default().fg(Color::Magenta).add_modifier(Modifier::BOLD)),
            Span::styled(format!("  {description}"), Style::default().fg(Color::DarkGray)),
        ]),
        Node::BackgroundTask {
            status,
            description,
            ..
        } => {
            let (marker, color) = match status {
                crate::graph::TaskStatus::Pending => ("○", Color::DarkGray),
                crate::graph::TaskStatus::Running => ("◉", Color::Yellow),
                crate::graph::TaskStatus::Completed => ("✓", Color::Green),
                crate::graph::TaskStatus::Failed => ("✗", Color::Red),
            };
            Line::from(vec![
                Span::styled(format!("{marker} "), Style::default().fg(color)),
                Span::styled(description.clone(), Style::default().fg(Color::White)),
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
    let items: Vec<ListItem> = history
        .iter()
        .filter_map(|node| {
            let (prefix, color) = match node {
                Node::Message { role, .. } => match role {
                    Role::User => ("U", Color::Cyan),
                    Role::Assistant => ("A", Color::Green),
                    Role::System => ("S", Color::DarkGray),
                },
                Node::SystemDirective { .. } => ("S", Color::DarkGray),
                _ => return None,
            };
            let content = node.content();
            let max_len = width.saturating_sub(3);
            let truncated: String = content.chars().take(max_len).collect();
            let line = Line::from(vec![
                Span::styled(format!("{prefix} "), Style::default().fg(color).add_modifier(Modifier::BOLD)),
                Span::styled(truncated, Style::default().fg(Color::White)),
            ]);
            Some(ListItem::new(line))
        })
        .collect();

    let list = List::new(items);
    frame.render_widget(list, inner);
}

fn is_git_file(node: &Node) -> bool {
    matches!(node, Node::GitFile { .. })
}

fn is_tool(node: &Node) -> bool {
    matches!(node, Node::Tool { .. })
}

fn is_background_task(node: &Node) -> bool {
    matches!(node, Node::BackgroundTask { .. })
}
