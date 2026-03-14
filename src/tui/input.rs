use crate::graph::{ConversationGraph, Node};
use crate::tui::state::FocusZone;
use crate::tui::{CompletionCandidate, TuiState};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

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

pub fn handle_key_event(
    key: KeyEvent,
    tui_state: &mut TuiState,
    graph: &ConversationGraph,
) -> Action {
    // ── Global keybindings (always active) ───────────────────────
    if key.modifiers.contains(KeyModifiers::CONTROL) {
        match key.code {
            KeyCode::Char('q') => return Action::Quit,
            KeyCode::Char('e') => {
                tui_state.tool_display = tui_state.tool_display.toggle();
                tui_state.render_cache.clear();
                return Action::None;
            }
            _ => {}
        }
    }

    // Number keys 1-3: switch tabs (only when TabContent focused).
    if tui_state.nav.focus == FocusZone::TabContent {
        if let KeyCode::Char(c @ '1'..='3') = key.code {
            if let Some(tab) = crate::tui::state::TopTab::from_number(c.to_digit(10).unwrap_or(0)) {
                tui_state.nav.active_tab = tab;
                return Action::None;
            }
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
        KeyCode::End => return Action::ScrollToBottom,
        _ => {}
    }
    // Everything else goes to the input handler (which handles Up/Down
    // overflow into scroll when the cursor is at the top/bottom of input).
    handle_input_key(key, tui_state, graph)
}

/// Handle keys when a tab's content area is focused.
/// Up/Down navigates the active tab's list or scrolls its content.
fn handle_tab_content_key(key: KeyEvent, tui_state: &mut TuiState) -> Action {
    let (offset, max) = match tui_state.nav.active_tab {
        crate::tui::state::TopTab::Work => (
            &mut tui_state.work_selected,
            tui_state.work_visible_count.saturating_sub(1),
        ),
        crate::tui::state::TopTab::Activity => (
            &mut tui_state.activity_scroll,
            tui_state.activity_total.saturating_sub(1),
        ),
        crate::tui::state::TopTab::Agents => (
            &mut tui_state.agents_scroll,
            tui_state.agents_total.saturating_sub(1),
        ),
    };
    match key.code {
        KeyCode::Up => *offset = offset.saturating_sub(1),
        KeyCode::Down => *offset = (*offset + 1).min(max),
        _ => {}
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

fn handle_input_key(key: KeyEvent, tui_state: &mut TuiState, graph: &ConversationGraph) -> Action {
    if let Some(action) = handle_autocomplete_key(&key, tui_state) {
        return action;
    }

    let action = match key.code {
        KeyCode::Enter
            if key.modifiers.contains(KeyModifiers::SHIFT)
                || key.modifiers.contains(KeyModifiers::ALT) =>
        {
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
        KeyCode::End => Action::ScrollToBottom,
        KeyCode::Esc if tui_state.pending_question_text.is_some() => Action::DismissQuestion,
        _ => Action::None,
    };

    // Re-filter autocomplete after text/cursor changes
    match key.code {
        KeyCode::Char(_) | KeyCode::Backspace | KeyCode::Left | KeyCode::Right | KeyCode::Enter => {
            update_autocomplete(tui_state, graph);
        }
        _ => {}
    }

    action
}

/// Detect `/` trigger and filter autocomplete candidates.
fn update_autocomplete(tui_state: &mut TuiState, graph: &ConversationGraph) {
    let chars: Vec<char> = tui_state.input_text.chars().collect();
    let cursor = tui_state.input_cursor;

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
