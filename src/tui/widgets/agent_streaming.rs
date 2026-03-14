//! Agent streaming display: renders the live agent output at the bottom
//! of the conversation panel (spinner, streaming text with reveal, cursor).

use crate::tui::widgets::display_helpers::{apply_reveal_fade, compute_styled_height};
use crate::tui::widgets::markdown::render_markdown;
use crate::tui::{AgentDisplayState, AgentVisualPhase, CURSOR_FRAMES};

use ratatui::prelude::*;

use super::conversation::MessageEntry;

/// Build a `MessageEntry::Streaming` for the active agent display.
/// Handles all three visual phases: preparing, executing tools, and streaming.
pub(super) fn build_agent_entry<'a>(
    display: &AgentDisplayState,
    status_message: Option<&String>,
    msg_content_width: usize,
) -> MessageEntry<'a> {
    match &display.phase {
        AgentVisualPhase::Preparing | AgentVisualPhase::ExecutingTools => {
            let status = status_message.map_or("Preparing...", String::as_str);
            let spinner = display.spinner_char();
            let styled = Text::from(Line::from(vec![
                Span::styled(format!("{spinner} "), Style::default().fg(Color::Green)),
                Span::styled(status.to_string(), Style::default().fg(Color::DarkGray)),
            ]));
            let height = compute_styled_height(&styled, msg_content_width, false);
            MessageEntry::Streaming {
                styled_text: styled,
                height,
            }
        }
        AgentVisualPhase::Streaming { text, is_thinking } => {
            build_streaming_entry(display, text, *is_thinking, msg_content_width)
        }
    }
}

/// Build the streaming text entry with reveal animation and cursor.
fn build_streaming_entry<'a>(
    display: &AgentDisplayState,
    text: &str,
    is_thinking: bool,
    msg_content_width: usize,
) -> MessageEntry<'a> {
    // Slice text at the revealed character boundary.
    let total_chars = text.chars().count();
    let reveal_count = display.revealed_chars.min(total_chars);
    let byte_offset = text
        .char_indices()
        .nth(reveal_count)
        .map_or(text.len(), |(i, _)| i);
    let revealed = &text[..byte_offset];

    let mut styled = render_markdown(revealed);

    // Apply fade-in gradient when there are unrevealed characters.
    if reveal_count < total_chars {
        apply_reveal_fade(&mut styled, 8);
    }

    if is_thinking && text.is_empty() {
        let spinner = display.spinner_char();
        styled.lines.push(Line::styled(
            format!("{spinner} Thinking..."),
            Style::default().fg(Color::DarkGray).italic(),
        ));
    }
    append_cursor(&mut styled, display.spinner_tick);
    let height = compute_styled_height(&styled, msg_content_width, false);
    MessageEntry::Streaming {
        styled_text: styled,
        height,
    }
}

/// Append a blinking block cursor to the last line of the styled text.
fn append_cursor(styled: &mut Text<'static>, tick: usize) {
    let cursor = CURSOR_FRAMES[tick % CURSOR_FRAMES.len()];
    let span = Span::styled(cursor, Style::default().fg(Color::Green));
    if let Some(last_line) = styled.lines.last_mut() {
        last_line.spans.push(span);
    } else {
        styled.lines.push(Line::from(span));
    }
}
