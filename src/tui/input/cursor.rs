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

#[cfg(test)]
mod tests {
    use super::cursor_line_col;

    /// Bug: cursor at position 0 returns nonzero line or column,
    /// causing the rendered cursor to appear in the wrong place.
    #[test]
    fn cursor_at_start_returns_zero_zero() {
        assert_eq!(cursor_line_col("hello\nworld", 0), (0, 0));
    }

    /// Bug: multi-line cursor math returns wrong line — cursor draws
    /// on the wrong visual row in the input box.
    #[test]
    fn cursor_on_second_line() {
        // "hello\nworld", cursor at 'o' in "world" (index 7)
        assert_eq!(cursor_line_col("hello\nworld", 7), (1, 1));
    }

    /// Bug: cursor positioned exactly at the newline character is
    /// attributed to the wrong line, causing off-by-one in rendering.
    #[test]
    fn cursor_at_newline_char() {
        // "abc\ndef", cursor at index 3 (the '\n' itself)
        assert_eq!(cursor_line_col("abc\ndef", 3), (0, 3));
    }
}
