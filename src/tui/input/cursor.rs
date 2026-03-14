//! Multiline cursor positioning helpers for the input box.

use crate::tui::TuiState;

/// Returns `(line_index, column)` for a character-indexed cursor position.
pub(crate) fn cursor_line_col(text: &str, cursor: usize) -> (usize, usize) {
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

pub(super) fn has_line_above(tui_state: &TuiState) -> bool {
    cursor_line_col(&tui_state.input_text, tui_state.input_cursor).0 > 0
}

pub(super) fn has_line_below(tui_state: &TuiState) -> bool {
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

pub(super) fn move_cursor_up(tui_state: &mut TuiState) {
    let (cur_line, cur_col) = cursor_line_col(&tui_state.input_text, tui_state.input_cursor);
    if cur_line == 0 {
        return;
    }
    let (start, len) = line_start_and_len(&tui_state.input_text, cur_line - 1);
    tui_state.input_cursor = start + cur_col.min(len);
}

pub(super) fn move_cursor_down(tui_state: &mut TuiState) {
    let (cur_line, cur_col) = cursor_line_col(&tui_state.input_text, tui_state.input_cursor);
    if cur_line + 1 >= line_count(&tui_state.input_text) {
        return;
    }
    let (start, len) = line_start_and_len(&tui_state.input_text, cur_line + 1);
    tui_state.input_cursor = start + cur_col.min(len);
}
