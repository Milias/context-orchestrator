//! Graph tab navigation: tree movement, section cycling, detail panel,
//! and edge following.
//!
//! All functions take `&mut TuiState` and mutate the explorer/inspector
//! state for the active graph section. Called from the top-level input
//! dispatcher when the Graph tab has focus.

use crossterm::event::{KeyCode, KeyEvent};

use crate::tui::state::{ExplorerFocus, GraphSection};
use crate::tui::TuiState;

use super::Action;

/// Handle a key event when the Graph tab content area is focused.
///
/// Routes to section cycling, tree navigation, or detail panel
/// interaction depending on the active explorer focus and key pressed.
pub fn handle_graph_key(key: KeyEvent, tui_state: &mut TuiState) -> Action {
    // Section cycling: `[` = previous, `]` = next.
    match key.code {
        KeyCode::Char('[') => {
            tui_state.nav.active_graph_section = tui_state.nav.active_graph_section.prev();
            return Action::None;
        }
        KeyCode::Char(']') => {
            tui_state.nav.active_graph_section = tui_state.nav.active_graph_section.next();
            return Action::None;
        }
        _ => {}
    }

    let section = tui_state.nav.active_graph_section;

    // Determine current sub-panel focus.
    let focus = tui_state
        .explorer
        .get(&section)
        .map_or(ExplorerFocus::Tree, |e| e.focus);

    match focus {
        ExplorerFocus::Tree => handle_tree_key(key, tui_state, section),
        ExplorerFocus::Detail => handle_detail_key(key, tui_state, section),
    }
}

/// Handle keys when the tree sub-panel is focused.
///
/// Supports vim-style and arrow-key navigation:
/// - `Up`/`k`: move selection up
/// - `Down`/`j`: move selection down
/// - `Space`: toggle collapse/expand on current node
/// - `Enter`/`l`/`Right`: expand collapsed node or switch focus to detail
/// - `h`/`Left`: collapse expanded node or move to parent (no-op at root)
/// - `d`: toggle focus to the detail sub-panel
fn handle_tree_key(key: KeyEvent, tui_state: &mut TuiState, section: GraphSection) -> Action {
    let Some(explorer) = tui_state.explorer.get_mut(&section) else {
        return Action::None;
    };

    match key.code {
        KeyCode::Up | KeyCode::Char('k') => {
            explorer.move_selection(-1);
        }
        KeyCode::Down | KeyCode::Char('j') => {
            explorer.move_selection(1);
        }
        KeyCode::Char(' ') => {
            // Deferred to caller — needs graph access to resolve selected UUID.
            return Action::ToggleCollapse;
        }
        KeyCode::Enter | KeyCode::Char('l') | KeyCode::Right => {
            // Deferred to caller — needs graph access to resolve selected UUID.
            return Action::ExpandOrFocusDetail;
        }
        KeyCode::Char('h') | KeyCode::Left => {
            // Deferred to caller — needs graph access to resolve selected UUID.
            return Action::CollapseNode;
        }
        KeyCode::Char('d') => {
            explorer.focus = ExplorerFocus::Detail;
        }
        _ => {}
    }

    Action::None
}

/// Handle keys when the detail sub-panel is focused.
///
/// Supports edge navigation and breadcrumb backtracking:
/// - `Up`/`k`: select previous edge
/// - `Down`/`j`: select next edge
/// - `Enter`/`l`/`Right`: follow the selected edge
/// - `Esc`/`h`/`Left`: return focus to tree (or pop breadcrumb)
/// - `d`: toggle focus back to the tree sub-panel
fn handle_detail_key(key: KeyEvent, tui_state: &mut TuiState, section: GraphSection) -> Action {
    match key.code {
        KeyCode::Up | KeyCode::Char('k') => {
            let edge_count = tui_state.edge_inspector.edges.len();
            if edge_count > 0 {
                let sel = &mut tui_state.edge_inspector.selected_edge;
                *sel = sel.saturating_sub(1);
            }
        }
        KeyCode::Down | KeyCode::Char('j') => {
            let edge_count = tui_state.edge_inspector.edges.len();
            if edge_count > 0 {
                let sel = &mut tui_state.edge_inspector.selected_edge;
                *sel = (*sel + 1).min(edge_count - 1);
            }
        }
        KeyCode::Enter | KeyCode::Char('l') | KeyCode::Right => {
            return Action::FollowEdge;
        }
        KeyCode::Esc | KeyCode::Char('h') | KeyCode::Left => {
            // If there is a breadcrumb trail, pop it. Otherwise return to tree.
            if tui_state.edge_inspector.trail.is_empty() {
                if let Some(explorer) = tui_state.explorer.get_mut(&section) {
                    explorer.focus = ExplorerFocus::Tree;
                }
            } else {
                return Action::PopBreadcrumb;
            }
        }
        KeyCode::Char('d') => {
            if let Some(explorer) = tui_state.explorer.get_mut(&section) {
                explorer.focus = ExplorerFocus::Tree;
            }
        }
        _ => {}
    }

    Action::None
}
