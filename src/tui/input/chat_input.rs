//! Chat panel input handling: readline keybindings, autocomplete, and text entry.
//!
//! Handles all keystroke processing for the conversation input buffer:
//! Ctrl+key readline bindings, Alt+key word movement, bare key text entry,
//! and autocomplete navigation.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::graph::ConversationGraph;
use crate::tui::TuiState;

use super::autocomplete;
use super::Action;

/// Core input handler with readline keybindings.
///
/// Modifier-aware keys (Ctrl+, Alt+) are checked first, then bare keys.
/// Text/cursor mutations are delegated to `InputBuffer` methods.
pub fn handle_input_key(
    key: KeyEvent,
    tui_state: &mut TuiState,
    graph: &ConversationGraph,
) -> Action {
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
        autocomplete::update(tui_state, graph);
    }

    action
}

/// Autocomplete popup keys. Returns `Some` if consumed, `None` to fall through.
fn handle_autocomplete_key(key: &KeyEvent, tui_state: &mut TuiState) -> Option<Action> {
    if !tui_state.autocomplete.active || tui_state.autocomplete.candidates.is_empty() {
        return None;
    }
    match key.code {
        KeyCode::Enter => {
            autocomplete::accept(tui_state);
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
