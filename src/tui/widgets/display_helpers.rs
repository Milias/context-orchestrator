use crate::graph::tool_types::ToolCallArguments;
use crate::graph::{ConversationGraph, Node};
use ratatui::prelude::*;
use std::borrow::Cow;
use std::fmt::Write;
use std::path::Path;

const MAX_RESULT_LINES: usize = 20;

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

pub fn format_scroll_indicator(offset: u16, max: u16) -> String {
    match () {
        () if max == 0 => String::new(),
        () if offset >= max => " [END] ".to_string(),
        () => format!(" [{}%] ", (u32::from(offset) * 100) / u32::from(max)),
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
