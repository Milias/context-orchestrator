use super::build_error_section;
use crate::graph::{ConversationGraph, Node};

/// Bug: `build_error_section` returns a section when no errors exist,
/// polluting the system prompt with an empty heading.
#[test]
fn test_returns_none_without_errors() {
    let graph = ConversationGraph::new("sys");
    assert!(build_error_section(&graph).is_none());
}

/// Bug: `build_error_section` does not include the error message in the
/// rendered section, so the LLM has no visibility into what went wrong.
#[test]
fn test_returns_section_with_error_message() {
    let mut graph = ConversationGraph::new("sys");
    let leaf = graph.active_leaf().unwrap();
    graph.record_api_error(leaf, "Bad request (400): tool_result mismatch".into());

    let section = build_error_section(&graph).expect("should produce a section");
    assert!(section.contains("## Recent API Errors"));
    assert!(section.contains("Bad request (400): tool_result mismatch"));
}

/// Bug: stale `ApiError` nodes persist after cleanup, causing the error
/// section to appear in future system prompts.
#[test]
fn test_returns_none_after_cleanup() {
    let mut graph = ConversationGraph::new("sys");
    let leaf = graph.active_leaf().unwrap();
    graph.record_api_error(leaf, "some error".into());
    assert!(build_error_section(&graph).is_some());

    graph.remove_nodes_by(|n| matches!(n, Node::ApiError { .. }));
    assert!(build_error_section(&graph).is_none());
}
