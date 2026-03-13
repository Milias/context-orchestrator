use super::*;

#[test]
fn plain_text_renders() {
    let text = render_markdown("Hello world");
    assert!(!text.lines.is_empty());
}

#[test]
fn heading_renders_with_prefix() {
    let text = render_markdown("# Title");
    let first_line = &text.lines[0];
    let content: String = first_line
        .spans
        .iter()
        .map(|s| s.content.as_ref())
        .collect();
    assert!(content.contains("# Title"));
}

#[test]
fn bold_text_has_bold_modifier() {
    let text = render_markdown("Hello **bold** world");
    let spans: Vec<&Span> = text.lines.iter().flat_map(|l| l.spans.iter()).collect();
    let bold_span = spans.iter().find(|s| s.content.contains("bold"));
    assert!(bold_span.is_some());
    assert!(bold_span
        .unwrap()
        .style
        .add_modifier
        .contains(Modifier::BOLD));
}

#[test]
fn inline_code_has_background() {
    let text = render_markdown("Use `code` here");
    let spans: Vec<&Span> = text.lines.iter().flat_map(|l| l.spans.iter()).collect();
    let code_span = spans.iter().find(|s| s.content.contains("code"));
    assert!(code_span.is_some());
    assert_eq!(code_span.unwrap().style.bg, Some(CODE_BG));
}

#[test]
fn code_block_renders() {
    let md = "```rust\nlet x = 1;\n```";
    let text = render_markdown(md);
    let all_content: String = text
        .lines
        .iter()
        .flat_map(|l| l.spans.iter())
        .map(|s| s.content.as_ref())
        .collect();
    assert!(all_content.contains("rust"));
    assert!(all_content.contains("let"));
}

#[test]
fn list_item_renders_bullet() {
    let text = render_markdown("- Item one\n- Item two");
    let all_content: String = text
        .lines
        .iter()
        .flat_map(|l| l.spans.iter())
        .map(|s| s.content.as_ref())
        .collect();
    assert!(all_content.contains('\u{2022}')); // bullet
    assert!(all_content.contains("Item one"));
}

#[test]
fn think_blocks_are_skipped() {
    let md = "<think>\nthinking...\n</think>\nVisible text";
    let text = render_markdown(md);
    let all_content: String = text
        .lines
        .iter()
        .flat_map(|l| l.spans.iter())
        .map(|s| s.content.as_ref())
        .collect();
    assert!(!all_content.contains("thinking..."));
    assert!(all_content.contains("Visible text"));
}

#[test]
fn empty_content_produces_empty_text() {
    let text = render_markdown("");
    assert!(text.lines.is_empty());
}

#[test]
fn bold_in_list_item() {
    let text = render_markdown("- **bold** item");
    let spans: Vec<&Span> = text.lines.iter().flat_map(|l| l.spans.iter()).collect();
    let bold_span = spans.iter().find(|s| s.content.contains("bold"));
    assert!(bold_span.is_some(), "Should have a span containing 'bold'");
    assert!(
        bold_span
            .unwrap()
            .style
            .add_modifier
            .contains(Modifier::BOLD),
        "bold span should have BOLD modifier"
    );
}

#[test]
fn bold_in_heading() {
    let text = render_markdown("## **Important**");
    let spans: Vec<&Span> = text.lines.iter().flat_map(|l| l.spans.iter()).collect();
    let bold_span = spans.iter().find(|s| s.content.contains("Important"));
    assert!(bold_span.is_some());
    assert!(bold_span
        .unwrap()
        .style
        .add_modifier
        .contains(Modifier::BOLD));
}

#[test]
fn bold_in_blockquote() {
    let text = render_markdown("> **quoted bold**");
    let spans: Vec<&Span> = text.lines.iter().flat_map(|l| l.spans.iter()).collect();
    let bold_span = spans.iter().find(|s| s.content.contains("quoted bold"));
    assert!(bold_span.is_some());
    assert!(bold_span
        .unwrap()
        .style
        .add_modifier
        .contains(Modifier::BOLD));
}

#[test]
fn inline_code_in_list_item() {
    let text = render_markdown("- Use `code` here");
    let spans: Vec<&Span> = text.lines.iter().flat_map(|l| l.spans.iter()).collect();
    let code_span = spans.iter().find(|s| s.content.contains("code"));
    assert!(code_span.is_some());
    assert_eq!(code_span.unwrap().style.bg, Some(CODE_BG));
}

#[test]
fn single_tilde_stripped_by_parser() {
    // streamdown_parser treats single `~` as strikethrough toggle,
    // stripping it from output. User messages bypass markdown rendering
    // to preserve trigger syntax like `~plan`.
    let text = render_markdown("~read_file run.sh");
    let all_content: String = text
        .lines
        .iter()
        .flat_map(|l| l.spans.iter())
        .map(|s| s.content.as_ref())
        .collect();
    // Parser strips the tilde — this documents the known behavior
    assert!(!all_content.contains('~'));
    assert!(all_content.contains("read_file"));
}

// ── Bold-wrapped code ───────────────────────────────────────────────

#[test]
fn bold_wrapped_code_has_both_styles() {
    // `**`CLAUDE.md`**` should render with bold + code styling.
    // The parser normally loses the bold context around code spans.
    let text = render_markdown("See **`CLAUDE.md`** for details");
    let spans: Vec<&Span> = text.lines.iter().flat_map(|l| l.spans.iter()).collect();
    let code_span = spans.iter().find(|s| s.content.contains("CLAUDE.md"));
    assert!(code_span.is_some(), "should have a CLAUDE.md span");
    let style = code_span.unwrap().style;
    assert_eq!(style.bg, Some(CODE_BG), "should have code background");
    assert!(
        style.add_modifier.contains(Modifier::BOLD),
        "should be bold — bold wrapper must not be lost"
    );
}

#[test]
fn code_between_bold_spans_inherits_bold() {
    // `**text `code` rest**` — the code span sits between bold spans
    // and should inherit the bold modifier via flush_inline post-processing.
    let text = render_markdown("**start `middle` end**");
    let spans: Vec<&Span> = text.lines.iter().flat_map(|l| l.spans.iter()).collect();
    let code_span = spans.iter().find(|s| s.content.contains("middle"));
    assert!(code_span.is_some(), "should have a 'middle' span");
    let style = code_span.unwrap().style;
    assert_eq!(style.bg, Some(CODE_BG), "should have code background");
    assert!(
        style.add_modifier.contains(Modifier::BOLD),
        "code between bold spans should inherit bold"
    );
}
