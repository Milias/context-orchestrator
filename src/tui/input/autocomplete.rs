//! Autocomplete logic for `/command` completion in the chat input.
//!
//! Detects `/` triggers, filters tool-name candidates from the graph,
//! and replaces the prefix with the selected completion on accept.

use crate::graph::{ConversationGraph, Node};
use crate::tui::{CompletionCandidate, TuiState};

/// Detect `/` trigger and filter autocomplete candidates.
///
/// Scans backwards from the cursor to find a `/` preceded by whitespace
/// or at the start of the input. Filters graph tool nodes whose name
/// starts with the typed prefix (case-insensitive).
pub(super) fn update(tui_state: &mut TuiState, graph: &ConversationGraph) {
    let chars: Vec<char> = tui_state.input.text().chars().collect();
    let cursor = tui_state.input.cursor();

    // Scan backwards from cursor to find `/`
    let before_cursor = &chars[..cursor];
    let mut slash_pos = None;
    for i in (0..before_cursor.len()).rev() {
        if before_cursor[i] == '/' {
            if i == 0 || before_cursor[i - 1].is_whitespace() {
                slash_pos = Some(i);
            }
            break;
        }
        if before_cursor[i].is_whitespace() {
            break;
        }
    }

    let Some(tpos) = slash_pos else {
        tui_state.autocomplete.active = false;
        return;
    };

    let prefix: String = before_cursor[tpos + 1..cursor].iter().collect();

    if prefix.contains(char::is_whitespace) {
        tui_state.autocomplete.active = false;
        return;
    }

    let prefix_lower = prefix.to_lowercase();
    let candidates: Vec<_> = graph
        .nodes_by(|n| matches!(n, Node::Tool { .. }))
        .into_iter()
        .filter_map(|n| {
            if let Node::Tool {
                name, description, ..
            } = n
            {
                if name.to_lowercase().starts_with(&prefix_lower) {
                    Some(CompletionCandidate {
                        name: name.clone(),
                        description: description.clone(),
                    })
                } else {
                    None
                }
            } else {
                None
            }
        })
        .collect();

    tui_state.autocomplete.active = true;
    tui_state.autocomplete.trigger_char = '/';
    tui_state.autocomplete.prefix = prefix;
    tui_state.autocomplete.selected = tui_state
        .autocomplete
        .selected
        .min(candidates.len().saturating_sub(1));
    tui_state.autocomplete.candidates = candidates;
}

/// Accept the selected completion: replace `/prefix` with `/name `.
///
/// Rebuilds the input text by substituting the `/prefix` region with
/// the selected candidate's full name, then repositions the cursor
/// after the inserted text.
pub(super) fn accept(tui_state: &mut TuiState) {
    let Some(candidate) = tui_state
        .autocomplete
        .candidates
        .get(tui_state.autocomplete.selected)
    else {
        return;
    };
    let replacement = format!("/{} ", candidate.name);

    let chars: Vec<char> = tui_state.input.text().chars().collect();
    let cursor = tui_state.input.cursor();

    // Find the slash position (scan backwards)
    let before_cursor = &chars[..cursor];
    let mut slash_pos = None;
    for i in (0..before_cursor.len()).rev() {
        if before_cursor[i] == '/' {
            slash_pos = Some(i);
            break;
        }
    }

    let Some(tpos) = slash_pos else {
        return;
    };

    // Build new text: everything before `/` + replacement + everything after cursor
    let before: String = chars[..tpos].iter().collect();
    let after: String = chars[cursor..].iter().collect();
    let new_text = format!("{before}{replacement}{after}");
    let new_cursor = tpos + replacement.chars().count();
    tui_state.input.set_text(new_text);
    // set_text puts cursor at end; adjust to after replacement.
    tui_state.input.set_cursor(new_cursor);
    tui_state.autocomplete.active = false;
}
