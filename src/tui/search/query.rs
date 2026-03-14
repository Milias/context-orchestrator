//! Search query parsing for the graph explorer.
//!
//! Queries support free-text matching plus structured filter prefixes:
//! `type:`, `status:`, `role:`, `tool:`. A leading `!` inverts the match.
//!
//! Examples:
//! - `refactor` -- matches any node whose content contains "refactor"
//! - `type:workitem status:active` -- work items with active status
//! - `!type:toolcall` -- everything except tool calls
//! - `role:assistant context` -- assistant messages containing "context"

use crate::graph::Role;

/// A node type filter matching the `Node` enum variants.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeTypeFilter {
    /// `Node::Message`
    Message,
    /// `Node::WorkItem`
    WorkItem,
    /// `Node::ToolCall`
    ToolCall,
    /// `Node::ToolResult`
    ToolResult,
    /// `Node::Question`
    Question,
    /// `Node::Answer`
    Answer,
    /// `Node::GitFile`
    GitFile,
    /// `Node::BackgroundTask`
    BackgroundTask,
    /// `Node::ApiError`
    ApiError,
    /// `Node::ContextBuildingRequest`
    ContextBuildingRequest,
}

impl NodeTypeFilter {
    /// Parse prefix for `type:` filter.
    const TYPE_PREFIX: &str = "type:";
    /// Parse prefix for `status:` filter.
    const STATUS_PREFIX: &str = "status:";
    /// Parse prefix for `role:` filter.
    const ROLE_PREFIX: &str = "role:";
    /// Parse prefix for `tool:` filter.
    const TOOL_PREFIX: &str = "tool:";

    /// Parse a type filter value (case-insensitive).
    /// Returns `None` for unrecognized type names.
    fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "message" | "msg" => Some(Self::Message),
            "workitem" | "work" | "task" | "plan" => Some(Self::WorkItem),
            "toolcall" | "tool_call" => Some(Self::ToolCall),
            "toolresult" | "tool_result" => Some(Self::ToolResult),
            "question" | "q" => Some(Self::Question),
            "answer" | "a" => Some(Self::Answer),
            "gitfile" | "git_file" | "file" => Some(Self::GitFile),
            "backgroundtask" | "background_task" | "bg" => Some(Self::BackgroundTask),
            "apierror" | "api_error" | "error" => Some(Self::ApiError),
            "contextbuildingrequest" | "context" | "ctx" => Some(Self::ContextBuildingRequest),
            _ => None,
        }
    }
}

/// Parsed search query with optional structured filters.
///
/// Free-text tokens are joined and matched case-insensitively against
/// `Node::content()`. Structured filters narrow by node type, status,
/// role, or tool name. `inverted` flips the entire match.
#[derive(Debug, Clone, Default)]
pub struct SearchQuery {
    /// Free-text portion (joined remaining tokens after filter extraction).
    pub text: String,
    /// Filter by node variant.
    pub node_type: Option<NodeTypeFilter>,
    /// Filter by status field (matched as lowercase string).
    pub status: Option<String>,
    /// Filter by message role.
    pub role: Option<Role>,
    /// Filter by tool name (for `ToolCall` nodes).
    pub tool_name: Option<String>,
    /// When `true`, the entire match is inverted (show non-matching nodes).
    pub inverted: bool,
}

impl SearchQuery {
    /// Whether this query has no filters at all (empty search).
    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
            && self.node_type.is_none()
            && self.status.is_none()
            && self.role.is_none()
            && self.tool_name.is_none()
    }
}

/// Parse a raw search string into a structured query.
///
/// Splits on whitespace, extracts `type:`, `status:`, `role:`, `tool:`
/// prefixes, and joins remaining tokens as free text. A leading `!`
/// on the entire input inverts the match.
pub fn parse_query(input: &str) -> SearchQuery {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return SearchQuery::default();
    }

    let (inverted, rest) = if let Some(stripped) = trimmed.strip_prefix('!') {
        (true, stripped.trim_start())
    } else {
        (false, trimmed)
    };

    let mut query = SearchQuery {
        inverted,
        ..SearchQuery::default()
    };
    let mut text_parts: Vec<&str> = Vec::new();

    for token in rest.split_whitespace() {
        if let Some(value) = token.strip_prefix(NodeTypeFilter::TYPE_PREFIX) {
            query.node_type = NodeTypeFilter::from_str(value);
        } else if let Some(value) = token.strip_prefix(NodeTypeFilter::STATUS_PREFIX) {
            query.status = Some(value.to_lowercase());
        } else if let Some(value) = token.strip_prefix(NodeTypeFilter::ROLE_PREFIX) {
            query.role = parse_role(value);
        } else if let Some(value) = token.strip_prefix(NodeTypeFilter::TOOL_PREFIX) {
            query.tool_name = Some(value.to_string());
        } else {
            text_parts.push(token);
        }
    }

    query.text = text_parts.join(" ");
    query
}

/// Parse a role filter value (case-insensitive).
fn parse_role(s: &str) -> Option<Role> {
    match s.to_lowercase().as_str() {
        "user" | "u" => Some(Role::User),
        "assistant" | "a" => Some(Role::Assistant),
        "system" | "s" => Some(Role::System),
        _ => None,
    }
}
