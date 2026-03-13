use ratatui::prelude::*;

/// Highlight `/tool_name` patterns in styled text with a distinct style.
/// Scans each span for `/word` tokens at word boundaries and splits them
/// into separately styled spans.
pub fn highlight_triggers(text: &mut Text<'static>) {
    let trigger_style = Style::default()
        .fg(Color::Magenta)
        .add_modifier(Modifier::BOLD);

    for line in &mut text.lines {
        let mut new_spans: Vec<Span<'static>> = Vec::new();
        for span in line.spans.drain(..) {
            split_triggers_in_span(span, trigger_style, &mut new_spans);
        }
        line.spans = new_spans;
    }
}

fn split_triggers_in_span(span: Span<'static>, trigger_style: Style, out: &mut Vec<Span<'static>>) {
    let text = span.content.as_ref();
    let base_style = span.style;
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    let mut last_flush = 0;

    while i < chars.len() {
        if chars[i] == '/'
            && (i == 0 || chars[i - 1].is_whitespace())
            && i + 1 < chars.len()
            && chars[i + 1].is_alphanumeric()
        {
            // Flush text before the slash
            if i > last_flush {
                let before: String = chars[last_flush..i].iter().collect();
                out.push(Span::styled(before, base_style));
            }
            // Collect the trigger token: / + word chars (alphanumeric or _)
            let start = i;
            i += 1; // skip ~
            while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                i += 1;
            }
            let token: String = chars[start..i].iter().collect();
            out.push(Span::styled(token, trigger_style));
            last_flush = i;
        } else {
            i += 1;
        }
    }

    // Flush remaining text
    if last_flush < chars.len() {
        if last_flush == 0 {
            // No triggers found — return original span unchanged
            out.push(span);
        } else {
            let rest: String = chars[last_flush..].iter().collect();
            out.push(Span::styled(rest, base_style));
        }
    } else if last_flush == 0 && chars.is_empty() {
        out.push(span);
    }
}
