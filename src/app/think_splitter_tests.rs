use super::ThinkSplitter;

/// Catches visible text being corrupted when no think tags are present.
#[test]
fn no_think_tags() {
    let mut s = ThinkSplitter::new();
    s.push("Hello world");
    let (visible, think) = s.finish();
    assert_eq!(visible, "Hello world");
    assert!(think.is_empty());
}

/// Catches think block content leaking into visible text or vice versa.
#[test]
fn single_think_block() {
    let mut s = ThinkSplitter::new();
    s.push("<think>reasoning</think>answer");
    let (visible, think) = s.finish();
    assert_eq!(visible, "answer");
    assert_eq!(think, "reasoning");
}

/// Catches partial tag handling failing when tags span multiple chunks.
#[test]
fn think_block_across_chunks() {
    let mut s = ThinkSplitter::new();
    s.push("<thi");
    s.push("nk>reas");
    s.push("oning</thi");
    s.push("nk>answer");
    let (visible, think) = s.finish();
    assert_eq!(visible, "answer");
    assert_eq!(think, "reasoning");
}

/// Catches second think block being dropped or merged incorrectly.
#[test]
fn multiple_think_blocks() {
    let mut s = ThinkSplitter::new();
    s.push("before<think>first</think>middle<think>second</think>after");
    let (visible, think) = s.finish();
    assert_eq!(visible, "beforemiddleafter");
    assert_eq!(think, "first\nsecond");
}

/// Catches unclosed think blocks being treated as visible text.
#[test]
fn unclosed_think_block() {
    let mut s = ThinkSplitter::new();
    s.push("visible<think>partial thinking");
    let (visible, think) = s.finish();
    assert_eq!(visible, "visible");
    assert_eq!(think, "partial thinking");
}

/// Bug: `saturating_sub(6)` on a buffer with multi-byte chars (e.g. emoji) lands
/// inside a char, panicking on slice. The safe offset must snap to a char boundary.
#[test]
fn multibyte_chars_dont_panic() {
    let mut s = ThinkSplitter::new();
    s.push(" Test 🎨\n\n##");
    let (visible, _) = s.finish();
    assert!(visible.contains("🎨"));
}

/// Catches `is_thinking()` returning wrong state during incremental parsing.
#[test]
fn is_thinking_state() {
    let mut s = ThinkSplitter::new();
    assert!(!s.is_thinking());
    s.push("<think>thinking");
    assert!(s.is_thinking());
    s.push("</think>done");
    assert!(!s.is_thinking());
}
