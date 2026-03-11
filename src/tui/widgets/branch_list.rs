use crate::graph::ConversationGraph;
use crate::tui::{Focus, TuiState};
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, HighlightSpacing, List, ListItem, ListState};

pub fn render(frame: &mut Frame, area: Rect, graph: &ConversationGraph, tui_state: &TuiState) {
    let branches = graph.branch_names();
    let active = graph.active_branch();

    let items: Vec<ListItem> = branches
        .iter()
        .map(|name| {
            let prefix = if name.as_str() == active { "> " } else { "  " };
            ListItem::new(format!("{}{}", prefix, name))
        })
        .collect();

    let highlight_style = if tui_state.focus == Focus::BranchList {
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::White)
    };

    let list = List::new(items)
        .block(Block::default().title("Branches").borders(Borders::ALL))
        .highlight_style(highlight_style)
        .highlight_spacing(HighlightSpacing::Always);

    let mut state = ListState::default();
    state.select(Some(tui_state.branch_list_selected));

    frame.render_stateful_widget(list, area, &mut state);
}
