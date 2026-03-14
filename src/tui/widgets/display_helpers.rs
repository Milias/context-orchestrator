use crate::graph::tool_types::ToolCallArguments;
use crate::graph::{ConversationGraph, Node};
use ratatui::prelude::*;
use std::borrow::Cow;
use std::fmt::Write;
use std::path::Path;

const MAX_RESULT_LINES: usize = 20;

/// Brightness factors for the fade-in gradient. Index 0 = char closest to the
/// reveal frontier (dimmest), index 7 = furthest faded char (almost full).
const FADE_FACTORS: [f32; 8] = [0.15, 0.15, 0.40, 0.40, 0.70, 0.70, 0.90, 0.90];

/// Dim the last `fade_width` characters of rendered text to create a fade-in
/// effect at the reveal frontier. Characters closest to the end are dimmest;
/// characters further back transition to full brightness.
pub fn apply_reveal_fade(text: &mut Text<'static>, fade_width: usize) {
    let mut remaining = fade_width;
    for line in text.lines.iter_mut().rev() {
        if remaining == 0 {
            break;
        }
        let old_spans: Vec<Span<'static>> = line.spans.drain(..).collect();
        let mut new_spans: Vec<Span<'static>> = Vec::new();

        for span in old_spans.into_iter().rev() {
            if remaining == 0 {
                new_spans.push(span);
                continue;
            }
            let char_count = span.content.chars().count();
            if char_count <= remaining {
                let factor = fade_factor(remaining, fade_width);
                new_spans.push(Span::styled(span.content, dim_style(span.style, factor)));
                remaining = remaining.saturating_sub(char_count);
            } else {
                // Split: last `remaining` chars get dimmed, rest stays normal
                let split_at = char_count - remaining;
                let byte_boundary = span
                    .content
                    .char_indices()
                    .nth(split_at)
                    .map_or(span.content.len(), |(i, _)| i);
                let normal_part: String = span.content[..byte_boundary].into();
                let dimmed_part: String = span.content[byte_boundary..].into();
                let factor = fade_factor(remaining, fade_width);
                new_spans.push(Span::styled(dimmed_part, dim_style(span.style, factor)));
                new_spans.push(Span::styled(normal_part, span.style));
                remaining = 0;
            }
        }

        new_spans.reverse();
        line.spans = new_spans;
    }
}

/// Look up the brightness factor for a character based on its position in the
/// fade zone. `remaining` = how many fade chars are left to process (starts at
/// `fade_width`, decreases). The frontier (remaining == `fade_width`) maps to
/// the dimmest factor; the oldest faded char (remaining == 1) is the brightest.
fn fade_factor(remaining: usize, fade_width: usize) -> f32 {
    // distance_from_frontier: 0 = right at the frontier (dimmest)
    let distance_from_frontier = fade_width.saturating_sub(remaining);
    FADE_FACTORS
        .get(distance_from_frontier)
        .copied()
        .unwrap_or(1.0)
}

/// Compute a dimmed version of a style by reducing foreground brightness.
/// For `Rgb` colors the hue is preserved. Named colors fall back to the
/// terminal `DIM` modifier.
pub(super) fn dim_style(style: Style, factor: f32) -> Style {
    match style.fg {
        Some(Color::Rgb(r, g, b)) => style.fg(Color::Rgb(
            dim_channel(r, factor),
            dim_channel(g, factor),
            dim_channel(b, factor),
        )),
        Some(Color::Reset) | None => {
            // Default foreground — assume ~white on dark terminal
            let v = dim_channel(240, factor);
            style.fg(Color::Rgb(v, v, v))
        }
        _ => style.add_modifier(Modifier::DIM),
    }
}

/// Scale a single colour channel by `factor`, clamped to `[0, 255]`.
fn dim_channel(channel: u8, factor: f32) -> u8 {
    let scaled = f32::from(channel) * factor;
    // factor is in [0.0, 1.0] and channel in [0, 255], so the product fits in
    // u8 after rounding. Clamp defensively for safety.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    // Justified: scaled ∈ [0.0, 255.0] because factor ∈ [0.0, 1.0] and channel ∈ [0, 255].
    {
        scaled.clamp(0.0, 255.0) as u8
    }
}

pub fn display_content<'a>(node: &'a Node, graph: &'a ConversationGraph) -> Cow<'a, str> {
    match node {
        Node::ToolCall { arguments, .. } => Cow::Owned(arguments.display_summary()),
        Node::ToolResult {
            content,
            tool_call_id,
            ..
        } => format_tool_result(content.text_content(), *tool_call_id, graph),
        _ => Cow::Borrowed(node.content()),
    }
}

fn format_tool_result<'a>(
    text: &'a str,
    tool_call_id: uuid::Uuid,
    graph: &'a ConversationGraph,
) -> Cow<'a, str> {
    let ext = file_read_extension(graph, tool_call_id);
    let line_count = text.lines().count();

    let (body, overflow) = if line_count > MAX_RESULT_LINES {
        let truncated: String = text
            .lines()
            .take(MAX_RESULT_LINES)
            .collect::<Vec<_>>()
            .join("\n");
        (truncated, Some(line_count - MAX_RESULT_LINES))
    } else {
        (text.to_string(), None)
    };

    let is_markdown = ext
        .as_deref()
        .is_some_and(|e| matches!(e, "md" | "markdown" | "mdx"));

    let mut result = match ext.as_deref() {
        Some(e) if !is_markdown => format!("```{e}\n{body}\n```"),
        _ => body,
    };

    if let Some(n) = overflow {
        let _ = write!(result, "\n[... {n} more lines]");
    }
    Cow::Owned(result)
}

fn file_read_extension(graph: &ConversationGraph, tool_call_id: uuid::Uuid) -> Option<String> {
    let tc = graph.node(tool_call_id)?;
    if let Node::ToolCall {
        arguments: ToolCallArguments::ReadFile { path },
        ..
    } = tc
    {
        Path::new(path.as_str())
            .extension()
            .and_then(|e| e.to_str())
            .map(str::to_string)
    } else {
        None
    }
}

/// Format the scroll indicator shown in the conversation panel border.
///
/// - `[AUTO]` when autoscroll is active (content follows new messages).
/// - `[END]` when manually scrolled to the bottom.
/// - `[42%]` when manually scrolled to a specific position.
pub fn format_scroll_indicator(offset: u16, max: u16, mode: crate::tui::ScrollMode) -> String {
    if max == 0 {
        return String::new();
    }
    if mode == crate::tui::ScrollMode::Auto {
        return " [AUTO] ".to_string();
    }
    if offset >= max {
        " [END] ".to_string()
    } else {
        format!(" [{}%] ", (u32::from(offset) * 100) / u32::from(max))
    }
}

/// Compute the rendered height of styled text within a given content width.
/// +2 for the message block border. +1 if `has_thinking`.
pub fn compute_styled_height(text: &Text<'_>, content_width: usize, has_thinking: bool) -> usize {
    if content_width == 0 {
        return 2;
    }
    let mut total_lines = 0usize;
    if has_thinking {
        total_lines += 1;
    }
    for line in &text.lines {
        let w = line.width();
        if w == 0 {
            total_lines += 1;
        } else {
            total_lines += w.div_ceil(content_width);
        }
    }
    if text.lines.is_empty() {
        total_lines = 1;
    }
    total_lines + 2
}
