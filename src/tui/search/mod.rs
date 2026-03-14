//! Search state and evaluation for the graph explorer.
//!
//! Provides a search bar that filters visible graph nodes using
//! structured query syntax (type/status/role/tool prefixes) and
//! free-text matching. Results are stored as a set of matching
//! node UUIDs, recomputed on every query change.

pub mod matcher;
pub mod query;

use std::collections::HashSet;
use uuid::Uuid;

use crate::graph::ConversationGraph;
use query::{parse_query, SearchQuery};

/// Whether the search applies to the current tab or all sections.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchScope {
    /// Search within the active graph section only.
    Tab,
    /// Search across all graph sections.
    Global,
}

/// Live search state for the graph explorer.
///
/// Holds the raw query text, parsed filters, and the set of node UUIDs
/// that match. Re-evaluating on every keystroke keeps the results fresh.
#[derive(Debug)]
pub struct SearchState {
    /// Raw query text as typed by the user.
    pub query_text: String,
    /// Character-level cursor position within `query_text`.
    pub cursor: usize,
    /// Parsed structured query (updated on every text change).
    pub parsed: SearchQuery,
    /// Whether this search is scoped to the current tab or global.
    pub scope: SearchScope,
    /// Node UUIDs matching the current query.
    pub matching_ids: HashSet<Uuid>,
}

impl SearchState {
    /// Create a new empty search state.
    pub fn new() -> Self {
        Self {
            query_text: String::new(),
            cursor: 0,
            parsed: SearchQuery::default(),
            scope: SearchScope::Tab,
            matching_ids: HashSet::new(),
        }
    }

    /// Insert a character at the current cursor position and re-evaluate.
    pub fn insert_char(&mut self, ch: char, graph: &ConversationGraph) {
        let byte_pos = char_to_byte_offset(&self.query_text, self.cursor);
        self.query_text.insert(byte_pos, ch);
        self.cursor += 1;
        self.reparse_and_evaluate(graph);
    }

    /// Delete the character before the cursor and re-evaluate.
    /// No-op if the cursor is at position 0.
    pub fn delete_char(&mut self, graph: &ConversationGraph) {
        if self.cursor == 0 {
            return;
        }
        self.cursor -= 1;
        let byte_pos = char_to_byte_offset(&self.query_text, self.cursor);
        self.query_text.remove(byte_pos);
        self.reparse_and_evaluate(graph);
    }

    /// Toggle scope between `Tab` and `Global`.
    pub fn toggle_scope(&mut self) {
        self.scope = match self.scope {
            SearchScope::Tab => SearchScope::Global,
            SearchScope::Global => SearchScope::Tab,
        };
    }

    /// Re-parse the query text and re-evaluate against the graph.
    pub fn reparse_and_evaluate(&mut self, graph: &ConversationGraph) {
        self.parsed = parse_query(&self.query_text);
        self.evaluate(graph);
    }

    /// Re-evaluate the parsed query against all graph nodes.
    /// Populates `matching_ids` with UUIDs of matching nodes.
    fn evaluate(&mut self, graph: &ConversationGraph) {
        self.matching_ids.clear();
        if self.parsed.is_empty() {
            return;
        }
        for node in graph.nodes_by(|_| true) {
            if matcher::matches_node(&self.parsed, node) {
                self.matching_ids.insert(node.id());
            }
        }
    }
}

/// Convert a character offset to a byte offset within a UTF-8 string.
/// Clamps to the string length if the character offset exceeds the char count.
fn char_to_byte_offset(s: &str, char_offset: usize) -> usize {
    s.char_indices()
        .nth(char_offset)
        .map_or(s.len(), |(byte_pos, _)| byte_pos)
}
