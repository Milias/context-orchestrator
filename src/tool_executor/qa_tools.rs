//! Stateless executors for Q/A tools.
//!
//! These return placeholder content. Graph mutations (creating Question nodes,
//! adding edges) happen in `app::qa::effects` during `handle_tool_call_completed`.

use crate::graph::tool::result::ToolResultContent;

use super::ToolExecutionResult;

/// Stateless executor for the `ask` tool. Returns placeholder text.
/// The actual Question node + edges are created as a side-effect in the task handler.
pub fn execute_ask(question: &str) -> ToolExecutionResult {
    ToolExecutionResult {
        content: ToolResultContent::text(format!("Question submitted: {question}")),
        is_error: false,
    }
}
