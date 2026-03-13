use super::super::markdown::CODE_BG;
use super::*;
use ratatui::prelude::Modifier;

#[test]
fn table_row_renders_bold() {
    // Table rows previously used Line::raw(), losing all inline markdown.
    let mut table = TableBuffer::new();
    table.set_header(&["Name".into(), "Desc".into()]);
    table.add_row(&["**bold**".into(), "text".into()]);

    let mut lines = Vec::new();
    table.flush(&mut lines);

    let spans: Vec<&Span> = lines.iter().flat_map(|l| l.spans.iter()).collect();
    let bold_span = spans.iter().find(|s| s.content.contains("bold"));
    assert!(bold_span.is_some(), "table row should render bold text");
    assert!(
        bold_span
            .unwrap()
            .style
            .add_modifier
            .contains(Modifier::BOLD),
        "bold in table cell should have BOLD modifier"
    );
}

#[test]
fn bold_code_in_table_cell() {
    // `**`file`**` inside a table cell should render with bold + code styling.
    let mut table = TableBuffer::new();
    table.set_header(&["File".into(), "Desc".into()]);
    table.add_row(&["**`run.sh`**".into(), "script".into()]);

    let mut lines = Vec::new();
    table.flush(&mut lines);

    let spans: Vec<&Span> = lines.iter().flat_map(|l| l.spans.iter()).collect();
    let code_span = spans.iter().find(|s| s.content.contains("run.sh"));
    assert!(code_span.is_some(), "should have a run.sh span");
    let style = code_span.unwrap().style;
    assert_eq!(style.bg, Some(CODE_BG), "should have code background");
    assert!(
        style.add_modifier.contains(Modifier::BOLD),
        "bold-wrapped code in table cell should be bold"
    );
}
