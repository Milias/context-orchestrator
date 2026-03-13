use ratatui::prelude::*;
use regex::Regex;
use streamdown_parser::{InlineElement, InlineParser, ParseEvent, Parser};
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

struct TableBuffer {
    header: Option<Vec<String>>,
    rows: Vec<Vec<String>>,
    num_cols: usize,
}

impl TableBuffer {
    fn new() -> Self {
        Self {
            header: None,
            rows: Vec::new(),
            num_cols: 0,
        }
    }

    fn set_header(&mut self, cells: &[String]) {
        self.num_cols = self.num_cols.max(cells.len());
        self.header = Some(cells.to_vec());
    }

    fn add_row(&mut self, cells: &[String]) {
        self.num_cols = self.num_cols.max(cells.len());
        self.rows.push(cells.to_vec());
    }

    fn flush(self, lines: &mut Vec<Line<'static>>) {
        // Parse all cells into spans so we can compute visible widths.
        let parsed_header: Option<Vec<Vec<Span<'static>>>> = self
            .header
            .as_ref()
            .map(|h| h.iter().map(|c| parse_inline_spans(c)).collect());
        let parsed_rows: Vec<Vec<Vec<Span<'static>>>> = self
            .rows
            .iter()
            .map(|row| row.iter().map(|c| parse_inline_spans(c)).collect())
            .collect();

        let mut col_widths = vec![0usize; self.num_cols];
        if let Some(header) = &parsed_header {
            for (i, spans) in header.iter().enumerate() {
                col_widths[i] = col_widths[i].max(span_width(spans));
            }
        }
        for row in &parsed_rows {
            for (i, spans) in row.iter().enumerate() {
                if i < col_widths.len() {
                    col_widths[i] = col_widths[i].max(span_width(spans));
                }
            }
        }

        if let Some(header) = parsed_header {
            let base = Style::default().bold().fg(Color::Cyan);
            lines.push(build_table_line(&header, &col_widths, Some(base)));
            let total: usize =
                col_widths.iter().sum::<usize>() + col_widths.len().saturating_sub(1) * 3;
            lines.push(Line::styled(
                "\u{2500}".repeat(total.max(1)),
                Style::default().fg(Color::DarkGray),
            ));
        }
        for row in &parsed_rows {
            lines.push(build_table_line(row, &col_widths, None));
        }
    }
}

fn span_width(spans: &[Span<'_>]) -> usize {
    spans.iter().map(|s| s.content.len()).sum()
}

fn build_table_line<'a>(
    cells: &[Vec<Span<'a>>],
    col_widths: &[usize],
    base_style: Option<Style>,
) -> Line<'a> {
    let mut row_spans: Vec<Span<'a>> = Vec::new();
    for (i, cell_spans) in cells.iter().enumerate() {
        if i > 0 {
            let sep = match base_style {
                Some(s) => Span::styled(" | ", s),
                None => Span::raw(" | "),
            };
            row_spans.push(sep);
        }
        let vis_w = span_width(cell_spans);
        for span in cell_spans {
            let styled = match base_style {
                Some(base) => Span::styled(span.content.clone(), base.patch(span.style)),
                None => span.clone(),
            };
            row_spans.push(styled);
        }
        let target = col_widths.get(i).copied().unwrap_or(0);
        if vis_w < target {
            let pad = " ".repeat(target - vis_w);
            let padded = match base_style {
                Some(s) => Span::styled(pad, s),
                None => Span::raw(pad),
            };
            row_spans.push(padded);
        }
    }
    Line::from(row_spans)
}

struct RenderContext {
    inline_spans: Vec<Span<'static>>,
    code_language: Option<String>,
    table: TableBuffer,
}

impl RenderContext {
    fn new() -> Self {
        Self {
            inline_spans: Vec::new(),
            code_language: None,
            table: TableBuffer::new(),
        }
    }

    fn flush_inline(&mut self, lines: &mut Vec<Line<'static>>) {
        if !self.inline_spans.is_empty() {
            // Code spans adjacent to bold spans should inherit bold (the parser
            // flattens `**text `code` rest**` into separate elements, losing the
            // bold context around the code span).
            for i in 1..self.inline_spans.len().saturating_sub(1) {
                if self.inline_spans[i].style.bg == Some(CODE_BG) {
                    let prev_bold = self.inline_spans[i - 1]
                        .style
                        .add_modifier
                        .contains(Modifier::BOLD);
                    let next_bold = self.inline_spans[i + 1]
                        .style
                        .add_modifier
                        .contains(Modifier::BOLD);
                    if prev_bold || next_bold {
                        self.inline_spans[i].style =
                            self.inline_spans[i].style.add_modifier(Modifier::BOLD);
                    }
                }
            }
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

/// Parse inline markdown in a content string and return styled spans.
///
/// Pre-processes `**`code`**` patterns before the inline parser sees them,
/// because the parser treats code spans as formatting-independent and drops
/// the surrounding bold markers.
fn parse_inline_spans(content: &str) -> Vec<Span<'static>> {
    static BOLD_CODE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\*\*`([^`]+)`\*\*").unwrap());

    let mut spans = Vec::new();
    let mut last_end = 0;

    for cap in BOLD_CODE.captures_iter(content) {
        let m = cap.get(0).unwrap();
        if m.start() > last_end {
            let before = &content[last_end..m.start()];
            for elem in &InlineParser::new().parse(before) {
                push_inline(&mut spans, elem);
            }
        }
        let code_text = &cap[1];
        spans.push(Span::styled(
            format!(" {code_text} "),
            Style::default().fg(Color::Yellow).bg(CODE_BG).bold(),
        ));
        last_end = m.end();
    }

    if last_end == 0 {
        for elem in &InlineParser::new().parse(content) {
            push_inline(&mut spans, elem);
        }
    } else if last_end < content.len() {
        for elem in &InlineParser::new().parse(&content[last_end..]) {
            push_inline(&mut spans, elem);
        }
    }
    spans
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
            let style = heading_style(*level);
            let mut spans = vec![Span::styled(format!("{prefix} "), style)];
            for span in parse_inline_spans(content) {
                spans.push(Span::styled(span.content, style.patch(span.style)));
            }
            lines.push(Line::from(spans));
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
            lines.push(Line::from(highlight_code_line(
                code,
                ctx.code_language.as_deref(),
            )));
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
            let mut spans = vec![Span::styled(
                format!("{pad}{marker} "),
                Style::default().fg(Color::Cyan),
            )];
            spans.extend(parse_inline_spans(content));
            lines.push(Line::from(spans));
        }
        ParseEvent::TableHeader(cells) => {
            ctx.flush_inline(lines);
            ctx.table.set_header(cells);
        }
        ParseEvent::HorizontalRule => {
            ctx.flush_inline(lines);
            lines.push(Line::styled(
                "\u{2500}".repeat(40),
                Style::default().fg(Color::DarkGray),
            ));
        }
        ParseEvent::TableRow(cells) => {
            ctx.table.add_row(cells);
        }
        ParseEvent::TableEnd => {
            let table = std::mem::replace(&mut ctx.table, TableBuffer::new());
            table.flush(lines);
        }
        ParseEvent::BlockquoteLine(text) => {
            let base = Style::default().fg(Color::Gray);
            let mut spans = vec![Span::styled(
                "\u{2502} ",
                Style::default().fg(Color::DarkGray),
            )];
            for span in parse_inline_spans(text) {
                spans.push(Span::styled(span.content, base.patch(span.style)));
            }
            lines.push(Line::from(spans));
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
                Span::styled(text.to_string(), Style::default().fg(fg).bg(CODE_BG))
            })
            .collect(),
        Err(_) => vec![Span::styled(
            code.to_string(),
            Style::default().fg(Color::White).bg(CODE_BG),
        )],
    }
}

#[cfg(test)]
#[path = "markdown_tests.rs"]
mod tests;
