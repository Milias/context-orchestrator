//! Self-contained text buffer with cursor, kill buffer, and editing operations.
//!
//! Centralizes all byte-offset arithmetic, cursor movement, and text mutation
//! that was previously scattered across `TuiState` fields and `handle_input_key`.

/// Text buffer with character-indexed cursor and kill buffer for readline operations.
///
/// All byte-offset arithmetic is internal — callers work exclusively with
/// character indices. The kill buffer stores text from the most recent
/// kill command (Ctrl+K/U/W, Alt+D) for yanking with Ctrl+Y.
#[derive(Debug)]
pub struct InputBuffer {
    text: String,
    cursor: usize,
    kill_buffer: String,
}

impl InputBuffer {
    /// Create an empty input buffer.
    pub fn new() -> Self {
        Self {
            text: String::new(),
            cursor: 0,
            kill_buffer: String::new(),
        }
    }

    /// The current text content.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Character-indexed cursor position.
    pub fn cursor(&self) -> usize {
        self.cursor
    }

    /// Whether the buffer contains no text.
    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    /// Number of characters (not bytes) in the buffer.
    pub fn char_count(&self) -> usize {
        self.text.chars().count()
    }

    /// Clear the buffer and reset cursor to 0.
    pub fn clear(&mut self) {
        self.text.clear();
        self.cursor = 0;
    }

    /// Extract trimmed text and clear the buffer. Returns the trimmed text.
    pub fn take_text(&mut self) -> String {
        let text = self.text.trim().to_string();
        self.clear();
        text
    }

    /// Replace buffer contents and move cursor to end.
    pub fn set_text(&mut self, text: String) {
        self.cursor = text.chars().count();
        self.text = text;
    }

    /// Set the cursor position directly (clamped to char count).
    /// Used by autocomplete to position the cursor after the replacement.
    pub fn set_cursor(&mut self, pos: usize) {
        self.cursor = pos.min(self.char_count());
    }

    // ── Insertion ──────────────────────────────────────────────────

    /// Insert a character at the cursor and advance it.
    pub fn insert_char(&mut self, c: char) {
        let offset = self.byte_offset();
        self.text.insert(offset, c);
        self.cursor += 1;
    }

    /// Insert a newline at the cursor and advance it.
    pub fn insert_newline(&mut self) {
        self.insert_char('\n');
    }

    // ── Deletion ───────────────────────────────────────────────────

    /// Delete the character before the cursor (Backspace).
    pub fn delete_backward(&mut self) {
        if self.cursor > 0 {
            self.cursor -= 1;
            let offset = self.byte_offset();
            self.text.remove(offset);
        }
    }

    /// Delete the character at the cursor (Delete / Ctrl+D).
    pub fn delete_forward(&mut self) {
        if self.cursor < self.char_count() {
            let offset = self.byte_offset();
            self.text.remove(offset);
        }
    }

    // ── Character movement ─────────────────────────────────────────

    /// Move cursor one character left.
    pub fn move_left(&mut self) {
        if self.cursor > 0 {
            self.cursor -= 1;
        }
    }

    /// Move cursor one character right.
    pub fn move_right(&mut self) {
        if self.cursor < self.char_count() {
            self.cursor += 1;
        }
    }

    // ── Line movement ──────────────────────────────────────────────

    /// Move cursor to the start of the current line (Ctrl+A / Home).
    pub fn move_line_start(&mut self) {
        let (line, _) = self.cursor_line_col();
        let (start, _) = self.line_start_and_len(line);
        self.cursor = start;
    }

    /// Move cursor to the end of the current line (Ctrl+E / End).
    pub fn move_line_end(&mut self) {
        let (line, _) = self.cursor_line_col();
        let (start, len) = self.line_start_and_len(line);
        self.cursor = start + len;
    }

    /// Move cursor up one line, preserving column. Returns `false` if already at top.
    pub fn move_up(&mut self) -> bool {
        let (cur_line, cur_col) = self.cursor_line_col();
        if cur_line == 0 {
            return false;
        }
        let (start, len) = self.line_start_and_len(cur_line - 1);
        self.cursor = start + cur_col.min(len);
        true
    }

    /// Move cursor down one line, preserving column. Returns `false` if already at bottom.
    pub fn move_down(&mut self) -> bool {
        let (cur_line, cur_col) = self.cursor_line_col();
        if cur_line + 1 >= self.line_count() {
            return false;
        }
        let (start, len) = self.line_start_and_len(cur_line + 1);
        self.cursor = start + cur_col.min(len);
        true
    }

    // ── Word movement ──────────────────────────────────────────────

    /// Move cursor forward by one word (Alt+F).
    /// Skips non-alphanumeric, then skips alphanumeric.
    pub fn move_word_forward(&mut self) {
        self.cursor = self.find_word_end();
    }

    /// Move cursor backward by one word (Alt+B).
    /// Skips non-alphanumeric, then skips alphanumeric.
    pub fn move_word_backward(&mut self) {
        self.cursor = self.find_word_start();
    }

    // ── Kill / yank ────────────────────────────────────────────────

    /// Kill text from cursor to end of current line (Ctrl+K).
    /// If cursor is at end of line, kills the newline instead.
    pub fn kill_to_end(&mut self) {
        let (line, _) = self.cursor_line_col();
        let (start, len) = self.line_start_and_len(line);
        let line_end = start + len;

        if self.cursor == line_end && self.cursor < self.char_count() {
            // At end of line: kill just the newline
            let offset = self.byte_offset();
            self.kill_buffer = "\n".to_string();
            self.text.remove(offset);
        } else if self.cursor < line_end {
            let from = self.byte_offset();
            let to = self.byte_offset_at(line_end);
            self.kill_buffer = self.text[from..to].to_string();
            self.text.replace_range(from..to, "");
        }
    }

    /// Kill text from cursor to start of current line (Ctrl+U).
    pub fn kill_to_start(&mut self) {
        let (line, _) = self.cursor_line_col();
        let (line_start, _) = self.line_start_and_len(line);

        if self.cursor > line_start {
            let from = self.byte_offset_at(line_start);
            let to = self.byte_offset();
            self.kill_buffer = self.text[from..to].to_string();
            self.text.replace_range(from..to, "");
            self.cursor = line_start;
        }
    }

    /// Kill the word before the cursor (Ctrl+W).
    /// Skips whitespace backward, then deletes the word.
    pub fn kill_word_backward(&mut self) {
        let target = self.find_word_start();
        if target < self.cursor {
            let from = self.byte_offset_at(target);
            let to = self.byte_offset();
            self.kill_buffer = self.text[from..to].to_string();
            self.text.replace_range(from..to, "");
            self.cursor = target;
        }
    }

    /// Kill the word after the cursor (Alt+D).
    /// Skips whitespace forward, then deletes the word.
    pub fn delete_word_forward(&mut self) {
        let target = self.find_word_end();
        if target > self.cursor {
            let from = self.byte_offset();
            let to = self.byte_offset_at(target);
            self.kill_buffer = self.text[from..to].to_string();
            self.text.replace_range(from..to, "");
        }
    }

    /// Insert the kill buffer contents at the cursor (Ctrl+Y).
    pub fn yank(&mut self) {
        if !self.kill_buffer.is_empty() {
            let offset = self.byte_offset();
            let killed = self.kill_buffer.clone();
            self.text.insert_str(offset, &killed);
            self.cursor += killed.chars().count();
        }
    }

    // ── Height computation ─────────────────────────────────────────

    /// Compute the number of visual lines the text occupies within `content_width`.
    ///
    /// Accounts for both explicit newlines and soft wrapping at the given width.
    /// Returns at least 1 (the empty buffer still has one cursor line).
    pub fn visual_line_count(&self, content_width: usize) -> usize {
        if content_width == 0 || self.text.is_empty() {
            return 1;
        }
        let mut total = 0usize;
        for line in self.text.split('\n') {
            let w = line.chars().count();
            if w == 0 {
                total += 1;
            } else {
                total += w.div_ceil(content_width);
            }
        }
        total
    }

    // ── Private helpers ────────────────────────────────────────────

    /// Convert the current character-indexed cursor to a byte offset.
    fn byte_offset(&self) -> usize {
        self.byte_offset_at(self.cursor)
    }

    /// Convert a character index to a byte offset.
    fn byte_offset_at(&self, char_idx: usize) -> usize {
        self.text
            .char_indices()
            .nth(char_idx)
            .map_or(self.text.len(), |(i, _)| i)
    }

    /// Returns `(line_index, column)` for the current cursor position.
    fn cursor_line_col(&self) -> (usize, usize) {
        let mut line = 0;
        let mut col = 0;
        for (i, ch) in self.text.chars().enumerate() {
            if i == self.cursor {
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

    /// Number of lines (newline count + 1).
    fn line_count(&self) -> usize {
        self.text.chars().filter(|&c| c == '\n').count() + 1
    }

    /// Returns `(char_start, char_len)` for the given line index.
    fn line_start_and_len(&self, target: usize) -> (usize, usize) {
        let mut line = 0;
        let mut start = 0;
        for (i, ch) in self.text.chars().enumerate() {
            if ch == '\n' {
                if line == target {
                    return (start, i - start);
                }
                line += 1;
                start = i + 1;
            }
        }
        (start, self.char_count() - start)
    }

    /// Find the start of the word before the cursor (for backward word operations).
    /// Emacs convention: skip non-word chars backward, then skip word chars backward.
    /// Crosses newlines — Ctrl+W at start of a line deletes back into the previous line.
    fn find_word_start(&self) -> usize {
        let chars: Vec<char> = self.text.chars().collect();
        let mut pos = self.cursor;
        // Skip non-alphanumeric backward (including newlines)
        while pos > 0 && !chars[pos - 1].is_alphanumeric() {
            pos -= 1;
        }
        // Skip alphanumeric backward
        while pos > 0 && chars[pos - 1].is_alphanumeric() {
            pos -= 1;
        }
        pos
    }

    /// Find the end of the word after the cursor (for forward word operations).
    /// Emacs convention: skip non-word chars forward, then skip word chars forward.
    /// Forward word movement crosses newlines (unlike backward, which stops at them).
    fn find_word_end(&self) -> usize {
        let chars: Vec<char> = self.text.chars().collect();
        let len = chars.len();
        let mut pos = self.cursor;
        // Skip non-alphanumeric forward (including newlines)
        while pos < len && !chars[pos].is_alphanumeric() {
            pos += 1;
        }
        // Skip alphanumeric forward
        while pos < len && chars[pos].is_alphanumeric() {
            pos += 1;
        }
        pos
    }
}

#[cfg(test)]
#[path = "buffer_tests.rs"]
mod tests;
