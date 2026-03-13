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

/// Handle key events when autocomplete popup is active.
/// Returns `Some(Action)` if the key was consumed, `None` to fall through.
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

fn handle_input_key(key: KeyEvent, tui_state: &mut TuiState) -> Action {
    if let Some(action) = handle_autocomplete_key(&key, tui_state) {
        return action;
    }

    let action = match key.code {
        KeyCode::Enter if key.modifiers.contains(KeyModifiers::ALT) => {
            let byte_offset = tui_state
                .input_text
                .char_indices()
                .nth(tui_state.input_cursor)
                .map_or(tui_state.input_text.len(), |(i, _)| i);
            tui_state.input_text.insert(byte_offset, '\n');
            tui_state.input_cursor += 1;
            Action::None
        }
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
        KeyCode::Up => {
            if has_line_above(tui_state) {
                move_cursor_up(tui_state);
                Action::None
            } else {
                Action::ScrollUp
            }
        }
        KeyCode::Down => {
            if has_line_below(tui_state) {
                move_cursor_down(tui_state);
                Action::None
            } else {
                Action::ScrollDown
            }
        }
        KeyCode::PageUp => Action::PageUp,
        KeyCode::PageDown => Action::PageDown,
        _ => Action::None,
    };

    // Re-filter autocomplete after text/cursor changes
    match key.code {
        KeyCode::Char(_) | KeyCode::Backspace | KeyCode::Left | KeyCode::Right | KeyCode::Enter => {
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

// ── Multiline cursor helpers ─────────────────────────────────

/// Returns `(line_index, column)` for a character-indexed cursor position.
pub(super) fn cursor_line_col(text: &str, cursor: usize) -> (usize, usize) {
    let mut line = 0;
    let mut col = 0;
    for (i, ch) in text.chars().enumerate() {
        if i == cursor {
            break;
        }
        if ch == '\n' {
            line += 1;
            col = 0;
        } else {
            col += 1;
        }
    }
    (line, col)
}

fn line_count(text: &str) -> usize {
    text.chars().filter(|&c| c == '\n').count() + 1
}

fn has_line_above(tui_state: &TuiState) -> bool {
    cursor_line_col(&tui_state.input_text, tui_state.input_cursor).0 > 0
}

fn has_line_below(tui_state: &TuiState) -> bool {
    let (line, _) = cursor_line_col(&tui_state.input_text, tui_state.input_cursor);
    line + 1 < line_count(&tui_state.input_text)
}

/// Returns `(char_start, char_len)` for the given line index.
fn line_start_and_len(text: &str, target: usize) -> (usize, usize) {
    let mut line = 0;
    let mut start = 0;
    for (i, ch) in text.chars().enumerate() {
        if ch == '\n' {
            if line == target {
                return (start, i - start);
            }
            line += 1;
            start = i + 1;
        }
    }
    (start, text.chars().count() - start)
}

fn move_cursor_up(tui_state: &mut TuiState) {
    let (cur_line, cur_col) = cursor_line_col(&tui_state.input_text, tui_state.input_cursor);
    if cur_line == 0 {
        return;
    }
    let (start, len) = line_start_and_len(&tui_state.input_text, cur_line - 1);
    tui_state.input_cursor = start + cur_col.min(len);
}

fn move_cursor_down(tui_state: &mut TuiState) {
    let (cur_line, cur_col) = cursor_line_col(&tui_state.input_text, tui_state.input_cursor);
    if cur_line + 1 >= line_count(&tui_state.input_text) {
        return;
    }
    let (start, len) = line_start_and_len(&tui_state.input_text, cur_line + 1);
    tui_state.input_cursor = start + cur_col.min(len);
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
