pub mod buffer;
mod cursor;

use crate::graph::{ConversationGraph, Node};
use crate::tui::state::FocusZone;
use crate::tui::{CompletionCandidate, TuiState};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

pub(crate) use cursor::cursor_line_col;

#[derive(Debug)]
pub enum Action {
    None,
    Quit,
    SendMessage(String),
    /// Dismiss the pending user question without answering.
    DismissQuestion,
    ScrollUp,
    ScrollDown,
    PageUp,
    PageDown,
    /// Jump to bottom and re-enable autoscroll.
    ScrollToBottom,
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
            accept_completion(tui_state);
            return Action::None;
        }
        tui_state.nav.focus = match tui_state.nav.focus {
            FocusZone::TabContent => FocusZone::ChatPanel,
            FocusZone::ChatPanel => FocusZone::TabContent,
        };
        return Action::None;
    }

    // ── Per-zone dispatch ────────────────────────────────────────
    match tui_state.nav.focus {
        FocusZone::ChatPanel => handle_chat_panel_key(key, tui_state, graph),
        FocusZone::TabContent => handle_tab_content_key(key, tui_state),
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
    handle_input_key(key, tui_state, graph)
}

/// Handle keys when a tab's content area is focused.
/// Up/Down scrolls through the overview's activity stream.
fn handle_tab_content_key(key: KeyEvent, tui_state: &mut TuiState) -> Action {
    match tui_state.nav.active_tab {
        crate::tui::state::TopTab::Overview => match key.code {
            KeyCode::Up => tui_state
                .overview_scroll
                .scroll_by(-1, tui_state.overview_max),
            KeyCode::Down => tui_state
                .overview_scroll
                .scroll_by(1, tui_state.overview_max),
            _ => {}
        },
        crate::tui::state::TopTab::Graph | crate::tui::state::TopTab::System => {}
    }
    Action::None
}

/// Autocomplete popup keys. Returns `Some` if consumed, `None` to fall through.
fn handle_autocomplete_key(key: &KeyEvent, tui_state: &mut TuiState) -> Option<Action> {
    if !tui_state.autocomplete.active || tui_state.autocomplete.candidates.is_empty() {
        return None;
    }
    match key.code {
        KeyCode::Enter => {
            accept_completion(tui_state);
            Some(Action::None)
        }
        KeyCode::Up => {
            let len = tui_state.autocomplete.candidates.len();
            tui_state.autocomplete.selected = (tui_state.autocomplete.selected + len - 1) % len;
            Some(Action::None)
        }
        KeyCode::Down => {
            let len = tui_state.autocomplete.candidates.len();
            tui_state.autocomplete.selected = (tui_state.autocomplete.selected + 1) % len;
            Some(Action::None)
        }
        KeyCode::Esc => {
            tui_state.autocomplete.active = false;
            Some(Action::None)
        }
        _ => None,
    }
}

/// Core input handler with readline keybindings.
///
/// Modifier-aware keys (Ctrl+, Alt+) are checked first, then bare keys.
/// Text/cursor mutations are delegated to `InputBuffer` methods.
fn handle_input_key(key: KeyEvent, tui_state: &mut TuiState, graph: &ConversationGraph) -> Action {
    if let Some(action) = handle_autocomplete_key(&key, tui_state) {
        return action;
    }

    let action = if let Some(a) = handle_ctrl_keys(&key, tui_state) {
        a
    } else if let Some(a) = handle_alt_keys(&key, tui_state) {
        a
    } else {
        handle_bare_keys(&key, tui_state)
    };

    // Re-filter autocomplete after text/cursor changes.
    if modifies_text_or_cursor(&key) {
        update_autocomplete(tui_state, graph);
    }

    action
}

/// Handle Ctrl+key bindings. Returns `None` only for non-Ctrl or non-Char events.
/// Unbound Ctrl+Char combinations are absorbed (not passed to bare-key handler)
/// to prevent control characters from being inserted into the buffer.
fn handle_ctrl_keys(key: &KeyEvent, tui_state: &mut TuiState) -> Option<Action> {
    if !key.modifiers.contains(KeyModifiers::CONTROL) {
        return None;
    }
    let input = &mut tui_state.input;
    match key.code {
        KeyCode::Char('a') => input.move_line_start(),
        KeyCode::Char('e') => input.move_line_end(),
        KeyCode::Char('f') => input.move_right(),
        KeyCode::Char('b') => input.move_left(),
        KeyCode::Char('d') => input.delete_forward(),
        KeyCode::Char('h') => input.delete_backward(),
        KeyCode::Char('k') => input.kill_to_end(),
        KeyCode::Char('u') => input.kill_to_start(),
        KeyCode::Char('w') => input.kill_word_backward(),
        KeyCode::Char('y') => input.yank(),
        // Absorb unbound Ctrl+Char to prevent inserting control characters.
        KeyCode::Char(_) => {}
        _ => return None,
    }
    Some(Action::None)
}

/// Handle Alt+key bindings. Returns `None` if not consumed.
fn handle_alt_keys(key: &KeyEvent, tui_state: &mut TuiState) -> Option<Action> {
    if !key.modifiers.contains(KeyModifiers::ALT) {
        return None;
    }
    let input = &mut tui_state.input;
    match key.code {
        KeyCode::Char('f') => input.move_word_forward(),
        KeyCode::Char('b') => input.move_word_backward(),
        KeyCode::Char('d') => input.delete_word_forward(),
        _ => return None,
    }
    Some(Action::None)
}

/// Handle bare keys (no modifiers, or Shift/Alt for Enter newline).
fn handle_bare_keys(key: &KeyEvent, tui_state: &mut TuiState) -> Action {
    match key.code {
        KeyCode::Enter
            if key
                .modifiers
                .intersects(KeyModifiers::SHIFT | KeyModifiers::ALT) =>
        {
            tui_state.input.insert_newline();
            Action::None
        }
        KeyCode::Enter => {
            let text = tui_state.input.take_text();
            if text.is_empty() {
                return Action::None;
            }
            tui_state.input_scroll = 0;
            tui_state.autocomplete.active = false;
            Action::SendMessage(text)
        }
        KeyCode::Char(c) => {
            tui_state.input.insert_char(c);
            Action::None
        }
        KeyCode::Backspace => {
            tui_state.input.delete_backward();
            Action::None
        }
        KeyCode::Delete => {
            tui_state.input.delete_forward();
            Action::None
        }
        KeyCode::Left => {
            tui_state.input.move_left();
            Action::None
        }
        KeyCode::Right => {
            tui_state.input.move_right();
            Action::None
        }
        KeyCode::Up => {
            if tui_state.input.move_up() {
                Action::None
            } else {
                Action::ScrollUp
            }
        }
        KeyCode::Down => {
            if tui_state.input.move_down() {
                Action::None
            } else {
                Action::ScrollDown
            }
        }
        KeyCode::Home => {
            tui_state.input.move_line_start();
            Action::None
        }
        KeyCode::End => {
            if tui_state.input.is_empty() {
                Action::ScrollToBottom
            } else {
                tui_state.input.move_line_end();
                Action::None
            }
        }
        KeyCode::PageUp => Action::PageUp,
        KeyCode::PageDown => Action::PageDown,
        KeyCode::Esc if tui_state.pending_question_text.is_some() => Action::DismissQuestion,
        _ => Action::None,
    }
}

/// Whether a key event could modify text or cursor position (triggers autocomplete refresh).
fn modifies_text_or_cursor(key: &KeyEvent) -> bool {
    matches!(
        key.code,
        KeyCode::Char(_)
            | KeyCode::Backspace
            | KeyCode::Delete
            | KeyCode::Left
            | KeyCode::Right
            | KeyCode::Up
            | KeyCode::Down
            | KeyCode::Enter
            | KeyCode::Home
            | KeyCode::End
    )
}

/// Detect `/` trigger and filter autocomplete candidates.
fn update_autocomplete(tui_state: &mut TuiState, graph: &ConversationGraph) {
    let chars: Vec<char> = tui_state.input.text().chars().collect();
    let cursor = tui_state.input.cursor();

    // Scan backwards from cursor to find `/`
    let before_cursor = &chars[..cursor];
    let mut slash_pos = None;
    for i in (0..before_cursor.len()).rev() {
        if before_cursor[i] == '/' {
            if i == 0 || before_cursor[i - 1].is_whitespace() {
                slash_pos = Some(i);
            }
            break;
        }
        if before_cursor[i].is_whitespace() {
            break;
        }
    }

    let Some(tpos) = slash_pos else {
        tui_state.autocomplete.active = false;
        return;
    };

    let prefix: String = before_cursor[tpos + 1..cursor].iter().collect();

    if prefix.contains(char::is_whitespace) {
        tui_state.autocomplete.active = false;
        return;
    }

    let prefix_lower = prefix.to_lowercase();
    let candidates: Vec<_> = graph
        .nodes_by(|n| matches!(n, Node::Tool { .. }))
        .into_iter()
        .filter_map(|n| {
            if let Node::Tool {
                name, description, ..
            } = n
            {
                if name.to_lowercase().starts_with(&prefix_lower) {
                    Some(CompletionCandidate {
                        name: name.clone(),
                        description: description.clone(),
                    })
                } else {
                    None
                }
            } else {
                None
            }
        })
        .collect();

    tui_state.autocomplete.active = true;
    tui_state.autocomplete.trigger_char = '/';
    tui_state.autocomplete.prefix = prefix;
    tui_state.autocomplete.selected = tui_state
        .autocomplete
        .selected
        .min(candidates.len().saturating_sub(1));
    tui_state.autocomplete.candidates = candidates;
}

/// Accept the selected completion: replace `/prefix` with `/name `.
fn accept_completion(tui_state: &mut TuiState) {
    let Some(candidate) = tui_state
        .autocomplete
        .candidates
        .get(tui_state.autocomplete.selected)
    else {
        return;
    };
    let replacement = format!("/{} ", candidate.name);

    let chars: Vec<char> = tui_state.input.text().chars().collect();
    let cursor = tui_state.input.cursor();

    // Find the slash position (scan backwards)
    let before_cursor = &chars[..cursor];
    let mut slash_pos = None;
    for i in (0..before_cursor.len()).rev() {
        if before_cursor[i] == '/' {
            slash_pos = Some(i);
            break;
        }
    }

    let Some(tpos) = slash_pos else {
        return;
    };

    // Build new text: everything before `/` + replacement + everything after cursor
    let before: String = chars[..tpos].iter().collect();
    let after: String = chars[cursor..].iter().collect();
    let new_text = format!("{before}{replacement}{after}");
    let new_cursor = tpos + replacement.chars().count();
    tui_state.input.set_text(new_text);
    // set_text puts cursor at end; adjust to after replacement.
    tui_state.input.set_cursor(new_cursor);
    tui_state.autocomplete.active = false;
}
