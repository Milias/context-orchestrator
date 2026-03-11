use ratatui::prelude::*;
use streamdown_parser::{InlineElement, ParseEvent, Parser};
use syntect::highlighting::{Theme, ThemeSet};
use syntect::parsing::SyntaxSet;

use std::sync::LazyLock;

static SYNTAX_SET: LazyLock<SyntaxSet> = LazyLock::new(SyntaxSet::load_defaults_newlines);
static THEME: LazyLock<Theme> = LazyLock::new(|| {
    let ts = ThemeSet::load_defaults();
    ts.themes["base16-eighties.dark"].clone()
});

/// Convert a markdown string to styled ratatui `Text`.
pub fn render_markdown(content: &str) -> Text<'static> {
    let mut parser = Parser::new();
    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut ctx = RenderContext::new();

    for line in content.lines() {
        let events = parser.parse_line(line);
        for event in events {
            process_event(&event, &mut lines, &mut ctx);
        }
    }

    for event in parser.finalize() {
        process_event(&event, &mut lines, &mut ctx);
    }

    // Flush any remaining inline spans
    ctx.flush_inline(&mut lines);

    Text::from(lines)
}

const CODE_BG: Color = Color::Rgb(40, 42, 54);

struct RenderContext {
    inline_spans: Vec<Span<'static>>,
    code_language: Option<String>,
    table_col_widths: Vec<usize>,
}

impl RenderContext {
    fn new() -> Self {
        Self {
            inline_spans: Vec::new(),
            code_language: None,
            table_col_widths: Vec::new(),
        }
    }

    fn flush_inline(&mut self, lines: &mut Vec<Line<'static>>) {
        if !self.inline_spans.is_empty() {
            lines.push(Line::from(std::mem::take(&mut self.inline_spans)));
        }
    }
}

// ── Heading colors by level ─────────────────────────────────────────

fn heading_style(level: u8) -> Style {
    let color = match level {
        1 => Color::Cyan,
        2 => Color::Green,
        3 => Color::Yellow,
        4 => Color::Blue,
        5 => Color::Magenta,
        _ => Color::White,
    };
    Style::default().fg(color).add_modifier(Modifier::BOLD)
}

fn format_table_row(cells: &[String], col_widths: &[usize]) -> String {
    cells
        .iter()
        .enumerate()
        .map(|(i, cell)| {
            let w = col_widths.get(i).copied().unwrap_or(cell.len());
            format!("{cell:<w$}")
        })
        .collect::<Vec<_>>()
        .join(" | ")
}

fn push_inline(spans: &mut Vec<Span<'static>>, elem: &InlineElement) {
    match elem {
        InlineElement::Text(s) => spans.push(Span::raw(s.clone())),
        InlineElement::Bold(s) => {
            spans.push(Span::styled(s.clone(), Style::default().bold()));
        }
        InlineElement::Italic(s) => {
            spans.push(Span::styled(s.clone(), Style::default().italic()));
        }
        InlineElement::BoldItalic(s) => {
            spans.push(Span::styled(s.clone(), Style::default().bold().italic()));
        }
        InlineElement::Underline(s) => {
            spans.push(Span::styled(
                s.clone(),
                Style::default().add_modifier(Modifier::UNDERLINED),
            ));
        }
        InlineElement::Strikeout(s) => {
            spans.push(Span::styled(
                s.clone(),
                Style::default().add_modifier(Modifier::CROSSED_OUT),
            ));
        }
        InlineElement::Code(s) => {
            spans.push(Span::styled(
                format!(" {s} "),
                Style::default().fg(Color::Yellow).bg(CODE_BG),
            ));
        }
        InlineElement::Link { text, .. } => {
            spans.push(Span::styled(
                text.clone(),
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::UNDERLINED),
            ));
        }
        InlineElement::Image { alt, .. } => {
            spans.push(Span::styled(
                format!("[img: {alt}]"),
                Style::default().fg(Color::DarkGray).italic(),
            ));
        }
        InlineElement::Footnote(s) => {
            spans.push(Span::styled(s.clone(), Style::default().fg(Color::Cyan)));
        }
    }
}

fn push_parse_inline(spans: &mut Vec<Span<'static>>, event: &ParseEvent) {
    match event {
        ParseEvent::Text(s) | ParseEvent::Prompt(s) => spans.push(Span::raw(s.clone())),
        ParseEvent::Bold(s) => spans.push(Span::styled(s.clone(), Style::default().bold())),
        ParseEvent::Italic(s) => spans.push(Span::styled(s.clone(), Style::default().italic())),
        ParseEvent::BoldItalic(s) => {
            spans.push(Span::styled(s.clone(), Style::default().bold().italic()));
        }
        ParseEvent::Underline(s) => spans.push(Span::styled(
            s.clone(),
            Style::default().add_modifier(Modifier::UNDERLINED),
        )),
        ParseEvent::Strikeout(s) => spans.push(Span::styled(
            s.clone(),
            Style::default().add_modifier(Modifier::CROSSED_OUT),
        )),
        ParseEvent::InlineCode(s) => spans.push(Span::styled(
            format!(" {s} "),
            Style::default().fg(Color::Yellow).bg(CODE_BG),
        )),
        ParseEvent::Link { text, .. } => {
            spans.push(Span::styled(
                text.clone(),
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::UNDERLINED),
            ));
        }
        ParseEvent::Image { alt, .. } => spans.push(Span::styled(
            format!("[img: {alt}]"),
            Style::default().fg(Color::DarkGray).italic(),
        )),
        ParseEvent::Footnote(s) => {
            spans.push(Span::styled(s.clone(), Style::default().fg(Color::Cyan)));
        }
        _ => {}
    }
}

fn is_inline_event(event: &ParseEvent) -> bool {
    matches!(
        event,
        ParseEvent::Text(_)
            | ParseEvent::Prompt(_)
            | ParseEvent::Bold(_)
            | ParseEvent::Italic(_)
            | ParseEvent::BoldItalic(_)
            | ParseEvent::Underline(_)
            | ParseEvent::Strikeout(_)
            | ParseEvent::InlineCode(_)
            | ParseEvent::Link { .. }
            | ParseEvent::Image { .. }
            | ParseEvent::Footnote(_)
    )
}

fn process_event(event: &ParseEvent, lines: &mut Vec<Line<'static>>, ctx: &mut RenderContext) {
    if is_inline_event(event) {
        push_parse_inline(&mut ctx.inline_spans, event);
        return;
    }

    match event {
        ParseEvent::BlockquoteStart { .. } => {
            ctx.flush_inline(lines);
        }
        ParseEvent::Newline => {
            if ctx.inline_spans.is_empty() {
                lines.push(Line::default());
            } else {
                ctx.flush_inline(lines);
            }
        }
        ParseEvent::EmptyLine => {
            ctx.flush_inline(lines);
            lines.push(Line::default());
        }
        ParseEvent::Heading { level, content } => {
            ctx.flush_inline(lines);
            let prefix = "#".repeat(*level as usize);
            lines.push(Line::styled(
                format!("{prefix} {content}"),
                heading_style(*level),
            ));
        }
        ParseEvent::InlineElements(elements) => {
            for elem in elements {
                push_inline(&mut ctx.inline_spans, elem);
            }
        }
        _ => process_block_event(event, lines, ctx),
    }
}

fn process_block_event(
    event: &ParseEvent,
    lines: &mut Vec<Line<'static>>,
    ctx: &mut RenderContext,
) {
    match event {
        ParseEvent::CodeBlockStart { language, .. } => {
            ctx.flush_inline(lines);
            ctx.code_language.clone_from(language);
            let label = language.as_deref().unwrap_or("code");
            lines.push(Line::styled(
                format!("\u{256d}\u{2500} {label} \u{2500}"),
                Style::default().fg(Color::DarkGray),
            ));
        }
        ParseEvent::CodeBlockLine(code) => {
            lines.push(Line::from(highlight_code_line(code, ctx.code_language.as_deref())));
        }
        ParseEvent::CodeBlockEnd => {
            lines.push(Line::styled(
                "\u{2570}\u{2500}".to_string(),
                Style::default().fg(Color::DarkGray),
            ));
            ctx.code_language = None;
        }
        ParseEvent::ListItem {
            indent,
            bullet,
            content,
        } => {
            ctx.flush_inline(lines);
            let pad = " ".repeat(*indent);
            let marker = match bullet {
                streamdown_parser::ListBullet::Ordered(n) => format!("{n}."),
                _ => "\u{2022}".to_string(),
            };
            lines.push(Line::from(vec![
                Span::styled(format!("{pad}{marker} "), Style::default().fg(Color::Cyan)),
                Span::raw(content.clone()),
            ]));
        }
        ParseEvent::TableHeader(cells) => {
            ctx.flush_inline(lines);
            ctx.table_col_widths = cells.iter().map(String::len).collect();
            lines.push(Line::styled(
                format_table_row(cells, &ctx.table_col_widths),
                Style::default().bold().fg(Color::Cyan),
            ));
        }
        ParseEvent::TableSeparator => {
            let total: usize = ctx.table_col_widths.iter().sum::<usize>()
                + ctx.table_col_widths.len().saturating_sub(1) * 3;
            let width = if total > 0 { total } else { 40 };
            lines.push(Line::styled(
                "\u{2500}".repeat(width),
                Style::default().fg(Color::DarkGray),
            ));
        }
        ParseEvent::HorizontalRule => {
            ctx.flush_inline(lines);
            lines.push(Line::styled(
                "\u{2500}".repeat(40),
                Style::default().fg(Color::DarkGray),
            ));
        }
        ParseEvent::TableRow(cells) => {
            for (i, cell) in cells.iter().enumerate() {
                if i < ctx.table_col_widths.len() {
                    ctx.table_col_widths[i] = ctx.table_col_widths[i].max(cell.len());
                }
            }
            lines.push(Line::raw(format_table_row(cells, &ctx.table_col_widths)));
        }
        ParseEvent::BlockquoteLine(text) => {
            lines.push(Line::from(vec![
                Span::styled("\u{2502} ", Style::default().fg(Color::DarkGray)),
                Span::styled(text.clone(), Style::default().fg(Color::Gray)),
            ]));
        }
        _ => {}
    }
}

// ── Syntax highlighting ─────────────────────────────────────────────

fn highlight_code_line(code: &str, language: Option<&str>) -> Vec<Span<'static>> {
    let syntax = language
        .and_then(|lang| SYNTAX_SET.find_syntax_by_token(lang))
        .unwrap_or_else(|| SYNTAX_SET.find_syntax_plain_text());

    let mut highlighter = syntect::easy::HighlightLines::new(syntax, &THEME);

    match highlighter.highlight_line(code, &SYNTAX_SET) {
        Ok(ranges) => ranges
            .into_iter()
            .map(|(style, text)| {
                let fg = Color::Rgb(style.foreground.r, style.foreground.g, style.foreground.b);
                Span::styled(
                    text.to_string(),
                    Style::default().fg(fg).bg(CODE_BG),
                )
            })
            .collect(),
        Err(_) => vec![Span::styled(
            code.to_string(),
            Style::default().fg(Color::White).bg(CODE_BG),
        )],
    }
}

#[cfg(test)]
mod tests {
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
}
