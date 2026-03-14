use super::InputBuffer;

/// Ctrl+W at column 0 crosses the newline and deletes the previous word.
#[test]
fn kill_word_at_line_start_crosses_newline() {
    let mut buf = InputBuffer::new();
    buf.set_text("hello\nworld".into());
    buf.cursor = 6; // start of "world"
    buf.kill_word_backward();
    assert_eq!(buf.text(), "world");
    assert_eq!(buf.cursor(), 0);
}

/// Ctrl+W must skip trailing whitespace then delete the word, not stop after whitespace.
#[test]
fn word_backward_skips_whitespace_then_word() {
    let mut buf = InputBuffer::new();
    buf.set_text("hello  world".into());
    buf.cursor = 12; // end
    buf.kill_word_backward();
    assert_eq!(buf.text(), "hello  ");
    assert_eq!(buf.cursor(), 7);
}

/// Kill buffer must persist across operations for Ctrl+Y to work.
#[test]
fn yank_inserts_last_killed_text() {
    let mut buf = InputBuffer::new();
    buf.set_text("hello world".into());
    buf.cursor = 11;
    buf.kill_word_backward(); // kills "world"
    assert_eq!(buf.text(), "hello ");
    buf.yank();
    assert_eq!(buf.text(), "hello world");
}

/// A 50-char line in a 20-wide box must produce 3 visual lines (ceil(50/20)).
#[test]
fn visual_line_count_wraps_long_line() {
    let mut buf = InputBuffer::new();
    buf.set_text("a".repeat(50));
    assert_eq!(buf.visual_line_count(20), 3);
}

/// "hello\n" has 2 visual lines — the empty line after the newline must be counted
/// so the cursor can be positioned there.
#[test]
fn visual_line_count_trailing_newline() {
    let mut buf = InputBuffer::new();
    buf.set_text("hello\n".into());
    assert_eq!(buf.visual_line_count(80), 2);
}

/// Word movement must not overshoot past `char_count()` or go below 0.
#[test]
fn move_word_stops_at_boundaries() {
    let mut buf = InputBuffer::new();
    buf.set_text("abc def".into());

    buf.cursor = 0;
    buf.move_word_backward();
    assert_eq!(buf.cursor(), 0, "backward from start stays at 0");

    buf.cursor = 7; // end
    buf.move_word_forward();
    assert_eq!(buf.cursor(), 7, "forward from end stays at end");
}

/// Byte-offset calculation must handle multi-byte characters correctly.
#[test]
fn insert_at_multibyte_boundary() {
    let mut buf = InputBuffer::new();
    buf.set_text("a\u{1F600}b".into()); // a😀b
    buf.cursor = 2; // after the emoji, before 'b'
    buf.insert_char('X');
    assert_eq!(buf.text(), "a\u{1F600}Xb");
    assert_eq!(buf.cursor(), 3);
}

/// Delete key at end of text must be a no-op, not panic.
#[test]
fn delete_forward_at_end_is_noop() {
    let mut buf = InputBuffer::new();
    buf.set_text("abc".into());
    buf.cursor = 3;
    buf.delete_forward();
    assert_eq!(buf.text(), "abc");
    assert_eq!(buf.cursor(), 3);
}

/// Ctrl+K at end of line kills the newline; Ctrl+K mid-line kills to end.
#[test]
fn kill_to_end_at_line_boundary_kills_newline() {
    let mut buf = InputBuffer::new();
    buf.set_text("hello\nworld".into());
    buf.cursor = 5; // end of "hello", before '\n'
    buf.kill_to_end();
    assert_eq!(buf.text(), "helloworld");
    assert_eq!(buf.cursor(), 5);
}

/// Ctrl+U kills from cursor to line start without touching previous lines.
#[test]
fn kill_to_start_on_second_line() {
    let mut buf = InputBuffer::new();
    buf.set_text("hello\nworld".into());
    buf.cursor = 9; // "wor|ld"
    buf.kill_to_start();
    assert_eq!(buf.text(), "hello\nld");
    assert_eq!(buf.cursor(), 6);
}

/// Alt+F when cursor is on a newline must advance past it to the next word.
/// Backward word movement stops at newlines, but forward movement crosses them.
#[test]
fn move_word_forward_crosses_newline() {
    let mut buf = InputBuffer::new();
    buf.set_text("hello\nworld".into());
    buf.cursor = 5; // on the '\n'
    buf.move_word_forward();
    assert_eq!(
        buf.cursor(),
        11,
        "should advance past newline to end of 'world'"
    );
}

/// All editing operations on an empty buffer must be no-ops, not panics.
#[test]
fn empty_buffer_operations_are_safe() {
    let mut buf = InputBuffer::new();
    buf.delete_forward();
    buf.delete_backward();
    buf.move_word_forward();
    buf.move_word_backward();
    buf.kill_to_end();
    buf.kill_to_start();
    buf.kill_word_backward();
    buf.delete_word_forward();
    buf.yank(); // empty kill buffer
    assert!(buf.is_empty());
    assert_eq!(buf.cursor(), 0);
    assert!(!buf.move_up());
    assert!(!buf.move_down());
}
