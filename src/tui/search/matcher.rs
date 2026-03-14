//! Node matching logic for search queries.
//!
//! Evaluates a [`SearchQuery`] against a [`Node`], checking each filter
//! field independently. All specified filters must match (AND semantics).
//! The `inverted` flag on the query flips the final result.

use crate::graph::node::{Node, Role};

use super::query::{NodeTypeFilter, SearchQuery};

/// Check whether a node matches a search query.
///
/// Each filter is checked independently:
/// - `node_type`: must match the `Node` variant
/// - `status`: case-insensitive match against status fields
/// - `role`: must match `Message` role
/// - `tool_name`: must match `ToolCall` tool name (case-insensitive)
/// - `text`: case-insensitive substring of `Node::content()`
///
/// All active filters use AND semantics. The `inverted` flag flips the result.
pub fn matches_node(query: &SearchQuery, node: &Node) -> bool {
    if query.is_empty() {
        return true;
    }

    let raw_match = check_type_filter(query, node)
        && check_status_filter(query, node)
        && check_role_filter(query, node)
        && check_tool_filter(query, node)
        && check_text_filter(query, node);

    if query.inverted {
        !raw_match
    } else {
        raw_match
    }
}

/// Check the `node_type` filter against the node's variant.
fn check_type_filter(query: &SearchQuery, node: &Node) -> bool {
    let Some(filter) = &query.node_type else {
        return true;
    };
    matches!(
        (filter, node),
        (NodeTypeFilter::Message, Node::Message { .. })
            | (NodeTypeFilter::WorkItem, Node::WorkItem { .. })
            | (NodeTypeFilter::ToolCall, Node::ToolCall { .. })
            | (NodeTypeFilter::ToolResult, Node::ToolResult { .. })
            | (NodeTypeFilter::Question, Node::Question { .. })
            | (NodeTypeFilter::Answer, Node::Answer { .. })
            | (NodeTypeFilter::GitFile, Node::GitFile { .. })
            | (NodeTypeFilter::BackgroundTask, Node::BackgroundTask { .. })
            | (NodeTypeFilter::ApiError, Node::ApiError { .. })
            | (
                NodeTypeFilter::ContextBuildingRequest,
                Node::ContextBuildingRequest { .. }
            )
    )
}

/// Check the `status` filter against status-bearing node variants.
fn check_status_filter(query: &SearchQuery, node: &Node) -> bool {
    let Some(status_filter) = &query.status else {
        return true;
    };
    let node_status = match node {
        Node::WorkItem { status, .. } => format!("{status:?}").to_lowercase(),
        Node::BackgroundTask { status, .. } => format!("{status:?}").to_lowercase(),
        Node::Question { status, .. } => format!("{status:?}").to_lowercase(),
        Node::ContextBuildingRequest { status, .. } => format!("{status:?}").to_lowercase(),
        // Nodes without a status field never match a status filter.
        _ => return false,
    };
    node_status == *status_filter
}

/// Check the `role` filter against `Message` nodes.
fn check_role_filter(query: &SearchQuery, node: &Node) -> bool {
    let Some(role_filter) = &query.role else {
        return true;
    };
    match node {
        Node::Message { role, .. } => role == role_filter,
        Node::SystemDirective { .. } => *role_filter == Role::System,
        // Non-message nodes do not have a role; filter excludes them.
        _ => false,
    }
}

/// Check the `tool_name` filter against `ToolCall` nodes.
fn check_tool_filter(query: &SearchQuery, node: &Node) -> bool {
    let Some(tool_filter) = &query.tool_name else {
        return true;
    };
    match node {
        Node::ToolCall { arguments, .. } => arguments
            .tool_name()
            .to_lowercase()
            .contains(&tool_filter.to_lowercase()),
        // Non-ToolCall nodes do not match a tool filter.
        _ => false,
    }
}

/// Check the free-text filter: case-insensitive substring of `Node::content()`.
fn check_text_filter(query: &SearchQuery, node: &Node) -> bool {
    if query.text.is_empty() {
        return true;
    }
    node.content()
        .to_lowercase()
        .contains(&query.text.to_lowercase())
}
