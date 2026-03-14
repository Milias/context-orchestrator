//! Build an error context section for injection into the system prompt.
//!
//! Queries `ApiError` nodes from the graph and renders them so the LLM
//! can see what went wrong on previous attempts and adapt its behavior.

use crate::graph::{ConversationGraph, Node};

/// Build the error section for the system prompt.
/// Returns `None` if no API errors exist (avoids injecting an empty section).
pub fn build_error_section(graph: &ConversationGraph) -> Option<String> {
    let errors: Vec<&Node> = graph.nodes_by(|n| matches!(n, Node::ApiError { .. }));

    if errors.is_empty() {
        return None;
    }

    let mut lines = vec![
        "## Recent API Errors".to_string(),
        "The following errors occurred on previous attempts. \
         Adjust your response to avoid triggering them \
         (e.g., use fewer tools per turn, shorter responses)."
            .to_string(),
    ];

    for node in &errors {
        if let Node::ApiError { message, .. } = node {
            lines.push(format!("- {message}"));
        }
    }

    Some(lines.join("\n"))
}

#[cfg(test)]
#[path = "error_context_tests.rs"]
mod tests;
