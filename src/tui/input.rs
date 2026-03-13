use crate::tui::{FocusPanel, TuiState};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

#[derive(Debug)]
pub enum Action {
    None,
    Quit,
    SendMessage(String),
    ScrollUp,
    ScrollDown,
    PageUp,
    PageDown,
}

pub fn handle_key_event(key: KeyEvent, tui_state: &mut TuiState) -> Action {
    // Global keybindings
    if key.modifiers.contains(KeyModifiers::CONTROL) {
        match key.code {
            KeyCode::Char('q') => return Action::Quit,
            KeyCode::Char('b') => {
                tui_state.context_panel_visible = !tui_state.context_panel_visible;
                if !tui_state.context_panel_visible && tui_state.focus == FocusPanel::ContextPanel {
                    tui_state.focus = FocusPanel::Input;
                }
                return Action::None;
            }
            _ => {}
        }
    }

    if key.code == KeyCode::Tab {
        // Autocomplete takes priority over focus toggle
        if tui_state.focus == FocusPanel::Input
            && tui_state.autocomplete.active
            && !tui_state.autocomplete.candidates.is_empty()
        {
            accept_completion(tui_state);
            return Action::None;
        }
        tui_state.focus = match tui_state.focus {
            FocusPanel::Input if tui_state.context_panel_visible => FocusPanel::ContextPanel,
            _ => FocusPanel::Input,
        };
        return Action::None;
    }

    match tui_state.focus {
        FocusPanel::Input => handle_input_key(key, tui_state),
        FocusPanel::ContextPanel => handle_context_panel_key(key, tui_state),
    }
}

fn handle_input_key(key: KeyEvent, tui_state: &mut TuiState) -> Action {
    // Autocomplete interception: Enter/Up/Down/Esc when popup is active
    if tui_state.autocomplete.active && !tui_state.autocomplete.candidates.is_empty() {
        match key.code {
            KeyCode::Enter => {
                accept_completion(tui_state);
                return Action::None;
            }
            KeyCode::Up => {
                let len = tui_state.autocomplete.candidates.len();
                tui_state.autocomplete.selected = (tui_state.autocomplete.selected + len - 1) % len;
                return Action::None;
            }
            KeyCode::Down => {
                let len = tui_state.autocomplete.candidates.len();
                tui_state.autocomplete.selected = (tui_state.autocomplete.selected + 1) % len;
                return Action::None;
            }
            KeyCode::Esc => {
                tui_state.autocomplete.active = false;
                return Action::None;
            }
            _ => {} // fall through to normal handling, then re-filter
        }
    }

    let action = match key.code {
        KeyCode::Enter => {
            let text = tui_state.input_text.trim().to_string();
            if text.is_empty() {
                return Action::None;
            }
            tui_state.input_text.clear();
            tui_state.input_cursor = 0;
            tui_state.autocomplete.active = false;
            Action::SendMessage(text)
        }
        KeyCode::Char(c) => {
            let byte_offset = tui_state
                .input_text
                .char_indices()
                .nth(tui_state.input_cursor)
                .map_or(tui_state.input_text.len(), |(i, _)| i);
            tui_state.input_text.insert(byte_offset, c);
            tui_state.input_cursor += 1;
            Action::None
        }
        KeyCode::Backspace => {
            if tui_state.input_cursor > 0 {
                tui_state.input_cursor -= 1;
                let byte_offset = tui_state
                    .input_text
                    .char_indices()
                    .nth(tui_state.input_cursor)
                    .map_or(tui_state.input_text.len(), |(i, _)| i);
                tui_state.input_text.remove(byte_offset);
            }
            Action::None
        }
        KeyCode::Left => {
            if tui_state.input_cursor > 0 {
                tui_state.input_cursor -= 1;
            }
            Action::None
        }
        KeyCode::Right => {
            if tui_state.input_cursor < tui_state.input_text.chars().count() {
                tui_state.input_cursor += 1;
            }
            Action::None
        }
        KeyCode::Up => Action::ScrollUp,
        KeyCode::Down => Action::ScrollDown,
        KeyCode::PageUp => Action::PageUp,
        KeyCode::PageDown => Action::PageDown,
        _ => Action::None,
    };

    // Re-filter autocomplete after text/cursor changes
    match key.code {
        KeyCode::Char(_) | KeyCode::Backspace | KeyCode::Left | KeyCode::Right => {
            update_autocomplete(tui_state);
        }
        _ => {}
    }

    action
}

/// Detect `/` trigger and filter autocomplete candidates.
fn update_autocomplete(tui_state: &mut TuiState) {
    let chars: Vec<char> = tui_state.input_text.chars().collect();
    let cursor = tui_state.input_cursor;

    // Scan backwards from cursor to find `/`
    let before_cursor = &chars[..cursor];
    let mut slash_pos = None;
    for i in (0..before_cursor.len()).rev() {
        if before_cursor[i] == '/' {
            // `/` must be at position 0 or preceded by whitespace
            if i == 0 || before_cursor[i - 1].is_whitespace() {
                slash_pos = Some(i);
            }
            break;
        }
        // If we hit whitespace before finding `/`, no active trigger
        if before_cursor[i].is_whitespace() {
            break;
        }
    }

    let Some(tpos) = slash_pos else {
        tui_state.autocomplete.active = false;
        return;
    };

    let prefix: String = before_cursor[tpos + 1..cursor].iter().collect();

    // If prefix contains whitespace, user is past the tool name (typing args)
    if prefix.contains(char::is_whitespace) {
        tui_state.autocomplete.active = false;
        return;
    }

    let prefix_lower = prefix.to_lowercase();
    let candidates: Vec<_> = tui_state
        .available_tools
        .iter()
        .filter(|t| t.name.to_lowercase().starts_with(&prefix_lower))
        .cloned()
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

    let chars: Vec<char> = tui_state.input_text.chars().collect();
    let cursor = tui_state.input_cursor;

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

    // Build new text: everything before `~` + replacement + everything after cursor
    let before: String = chars[..tpos].iter().collect();
    let after: String = chars[cursor..].iter().collect();
    tui_state.input_text = format!("{before}{replacement}{after}");
    tui_state.input_cursor = tpos + replacement.chars().count();
    tui_state.autocomplete.active = false;
}

fn handle_context_panel_key(key: KeyEvent, tui_state: &mut TuiState) -> Action {
    match key.code {
        KeyCode::Left => {
            tui_state.context_tab = tui_state.context_tab.prev();
            tui_state.context_list_offset = 0;
        }
        KeyCode::Right => {
            tui_state.context_tab = tui_state.context_tab.next();
            tui_state.context_list_offset = 0;
        }
        KeyCode::Up => {
            tui_state.context_list_offset = tui_state.context_list_offset.saturating_sub(1);
        }
        KeyCode::Down => {
            tui_state.context_list_offset += 1;
        }
        _ => {}
    }
    Action::None
}
