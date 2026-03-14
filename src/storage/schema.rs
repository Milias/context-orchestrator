//! Domain types and SQL schema for the analytics store.

// ── Domain types ────────────────────────────────────────────────────

/// Direction of token flow in an LLM interaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenDirection {
    /// Tokens sent to the model (prompt / input).
    Input,
    /// Tokens received from the model (completion / output).
    Output,
}

impl TokenDirection {
    /// Returns the string stored in the `direction` SQL column.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Input => "input",
            Self::Output => "output",
        }
    }

    /// Parse a SQL column value back into an enum variant.
    pub(crate) fn from_str(s: &str) -> Option<Self> {
        match s {
            "input" => Some(Self::Input),
            "output" => Some(Self::Output),
            _ => None,
        }
    }
}

/// A single token-usage event to be recorded in the analytics store.
#[derive(Debug, Clone)]
pub struct TokenEvent {
    /// Conversation that produced this event.
    pub conversation_id: String,
    /// Whether these are input or output tokens.
    pub direction: TokenDirection,
    /// Number of tokens in this event.
    pub tokens: u32,
    /// Model used (e.g. `"claude-sonnet-4-6"`). `None` for user-message counts.
    pub model: Option<String>,
}

/// Aggregated lifetime token totals across all conversations.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TokenTotals {
    /// Total input tokens ever recorded.
    pub input: u64,
    /// Total output tokens ever recorded.
    pub output: u64,
}

// ── SQL ─────────────────────────────────────────────────────────────

/// DDL for the `token_events` table.
///
/// `INTEGER PRIMARY KEY` is `SQLite`'s rowid alias and auto-increments
/// without the `AUTOINCREMENT` keyword. `created_at` is ISO 8601 text
/// (`SQLite` has no native timestamp type).
pub const CREATE_TOKEN_EVENTS: &str = "
    CREATE TABLE IF NOT EXISTS token_events (
        id INTEGER PRIMARY KEY,
        conversation_id TEXT NOT NULL,
        direction TEXT NOT NULL,
        tokens INTEGER NOT NULL,
        model TEXT,
        created_at TEXT NOT NULL
    )";

/// Insert a single token event.
pub const INSERT_TOKEN_EVENT: &str = "
    INSERT INTO token_events (conversation_id, direction, tokens, model, created_at)
    VALUES (?1, ?2, ?3, ?4, ?5)";

/// Aggregate totals grouped by direction.
pub const SUM_BY_DIRECTION: &str = "
    SELECT direction, COALESCE(SUM(tokens), 0) FROM token_events GROUP BY direction";
