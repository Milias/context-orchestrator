//! Input handling for the TUI: key dispatch, readline, graph navigation.
//!
//! Top-level dispatcher routes keys to per-zone handlers. Global bindings
//! (Ctrl+Q, Tab for focus switching) are checked first. Then:
//! - `ChatPanel` → readline input with autocomplete
//! - `TabContent` → tab switching, search, per-tab navigation

mod autocomplete;
pub mod buffer;
mod chat_input;
mod cursor;
mod graph_nav;

use crate::graph::ConversationGraph;
use crate::tui::state::FocusZone;
use crate::tui::TuiState;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

pub(crate) use cursor::cursor_line_col;

/// Actions produced by the input handler for the event loop to execute.
///
/// Simple mutations (tab switching, selection movement) are applied
/// directly in the handler. Actions that require the conversation graph
/// or cross-cutting side effects are returned here for the caller.
#[derive(Debug)]
pub enum Action {
    /// No-op — key was absorbed or produced only local state changes.
    None,
    /// Quit the application.
    Quit,
    /// Submit the user's typed message.
    SendMessage(String),
    /// Dismiss the pending user question without answering.
    DismissQuestion,
    /// Scroll conversation up by one line.
    ScrollUp,
    /// Scroll conversation down by one line.
    ScrollDown,
    /// Scroll conversation up by one page.
    PageUp,
    /// Scroll conversation down by one page.
    PageDown,
    /// Jump to bottom and re-enable autoscroll.
    ScrollToBottom,
    /// Toggle collapse/expand on the selected tree node (Graph tab).
    ToggleCollapse,
    /// Expand a collapsed node, or shift focus to the detail panel (Graph tab).
    ExpandOrFocusDetail,
    /// Collapse the selected node if expanded (Graph tab).
    CollapseNode,
    /// Follow the currently selected edge in the detail panel (Graph tab).
    FollowEdge,
    /// Pop one breadcrumb from the edge inspector trail (Graph tab).
    PopBreadcrumb,
}

/// Top-level key dispatcher: global bindings, then per-zone dispatch.
pub fn handle_key_event(
    key: KeyEvent,
    tui_state: &mut TuiState,
    graph: &ConversationGraph,
) -> Action {
    // ── Global keybindings (always active) ───────────────────────
    if key.modifiers.contains(KeyModifiers::CONTROL) {
        match key.code {
            KeyCode::Char('q') => return Action::Quit,
            // Ctrl+E: tool toggle when NOT in ChatPanel; falls through to
            // readline end-of-line when ChatPanel is focused.
            KeyCode::Char('e') if tui_state.nav.focus != FocusZone::ChatPanel => {
                tui_state.tool_display = tui_state.tool_display.toggle();
                tui_state.render_cache.clear();
                return Action::None;
            }
            _ => {}
        }
    }

    // Tab key: toggle between TabContent and ChatPanel.
    if key.code == KeyCode::Tab {
        // Autocomplete takes priority when chat panel is focused.
        if tui_state.nav.focus == FocusZone::ChatPanel
            && tui_state.autocomplete.active
            && !tui_state.autocomplete.candidates.is_empty()
        {
            autocomplete::accept(tui_state);
            return Action::None;
        }
        tui_state.nav.focus = match tui_state.nav.focus {
            FocusZone::TabContent => FocusZone::ChatPanel,
            FocusZone::ChatPanel => FocusZone::TabContent,
        };
        return Action::None;
    }

    // ── Search mode intercept ────────────────────────────────────
    if tui_state.search.is_some() {
        return handle_search_key(key, tui_state, graph);
    }

    // ── Per-zone dispatch ────────────────────────────────────────
    match tui_state.nav.focus {
        FocusZone::ChatPanel => handle_chat_panel_key(key, tui_state, graph),
        FocusZone::TabContent => handle_tab_content_key(key, tui_state, graph),
    }
}

/// Chat panel keys: typing goes to input, scroll keys scroll conversation.
fn handle_chat_panel_key(
    key: KeyEvent,
    tui_state: &mut TuiState,
    graph: &ConversationGraph,
) -> Action {
    // Scroll keys that always scroll the conversation.
    match key.code {
        KeyCode::PageUp => return Action::PageUp,
        KeyCode::PageDown => return Action::PageDown,
        _ => {}
    }
    // Everything else goes to the input handler (which handles Up/Down
    // overflow into scroll when the cursor is at the top/bottom of input).
    chat_input::handle_input_key(key, tui_state, graph)
}

/// Handle keys when a tab's content area is focused.
///
/// `/` activates search. Number keys (`1`-`3`) switch tabs. Remaining
/// keys are dispatched to the active tab's handler.
fn handle_tab_content_key(
    key: KeyEvent,
    tui_state: &mut TuiState,
    graph: &ConversationGraph,
) -> Action {
    use crate::tui::state::TopTab;

    // `/` activates the search bar (unless modified).
    if key.code == KeyCode::Char('/')
        && !key
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
    {
        tui_state.search = Some(crate::tui::search::SearchState::new());
        if let Some(search) = &mut tui_state.search {
            search.reparse_and_evaluate(graph);
        }
        return Action::None;
    }

    // Number keys switch tabs (unmodified only).
    if !key
        .modifiers
        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
    {
        if let Some(tab) = match key.code {
            KeyCode::Char('1') => Some(TopTab::Overview),
            KeyCode::Char('2') => Some(TopTab::Graph),
            KeyCode::Char('3') => Some(TopTab::System),
            _ => None,
        } {
            tui_state.nav.active_tab = tab;
            return Action::None;
        }
    }

    // Delegate to per-tab handlers.
    match tui_state.nav.active_tab {
        TopTab::Overview => handle_overview_key(key, tui_state),
        TopTab::Graph => graph_nav::handle_graph_key(key, tui_state),
        TopTab::System => handle_system_key(key, tui_state),
    }
}

/// Handle keys specific to the Overview tab.
///
/// Up/Down scrolls the activity stream.
fn handle_overview_key(key: KeyEvent, tui_state: &mut TuiState) -> Action {
    match key.code {
        KeyCode::Up => tui_state
            .overview_scroll
            .scroll_by(-1, tui_state.overview_max),
        KeyCode::Down => tui_state
            .overview_scroll
            .scroll_by(1, tui_state.overview_max),
        _ => {}
    }
    Action::None
}

/// Handle keys specific to the System tab.
///
/// Currently a placeholder — the System tab is read-only.
fn handle_system_key(_key: KeyEvent, _tui_state: &mut TuiState) -> Action {
    Action::None
}

/// Handle keys when the search bar is active.
///
/// Routes character input to the search query, handles backspace/escape,
/// and passes Ctrl+G for scope toggling. All other keys are ignored to
/// prevent accidental tab/panel navigation while searching.
fn handle_search_key(
    key: KeyEvent,
    tui_state: &mut TuiState,
    graph: &ConversationGraph,
) -> Action {
    // Ctrl+G: toggle scope (Tab vs Global).
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('g') {
        if let Some(search) = &mut tui_state.search {
            search.toggle_scope();
        }
        return Action::None;
    }

    // Ctrl+Q still quits even in search mode.
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('q') {
        return Action::Quit;
    }

    match key.code {
        KeyCode::Esc => {
            tui_state.search = None;
        }
        KeyCode::Char(c)
            if !key
                .modifiers
                .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
        {
            if let Some(search) = &mut tui_state.search {
                search.insert_char(c, graph);
            }
        }
        KeyCode::Backspace => {
            if let Some(search) = &mut tui_state.search {
                search.delete_char(graph);
            }
        }
        // Absorb other keys while searching — do not pass through.
        _ => {}
    }
    Action::None
}
