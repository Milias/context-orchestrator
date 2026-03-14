//! Free-standing cursor helper used by the input box renderer.
//!
//! The bulk of cursor logic now lives in [`super::buffer::InputBuffer`].
//! This module retains only the pure function needed by `input_box::render`,
//! which takes `(&str, usize)` rather than borrowing the whole `InputBuffer`.

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
