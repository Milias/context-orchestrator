//! Stateless executors for Q/A tools.
//!
//! These return placeholder content. Graph mutations (creating Question nodes,
//! adding edges) happen in `app::qa::effects` during `handle_tool_call_completed`.

use crate::graph::tool::result::ToolResultContent;
use uuid::Uuid;

use super::ToolExecutionResult;

/// Stateless executor for the `ask` tool. Returns placeholder text.
/// The actual Question node + edges are created as a side-effect in the task handler.
pub fn execute_ask(question: &str) -> ToolExecutionResult {
    ToolExecutionResult {
        content: ToolResultContent::text(format!("Question submitted: {question}")),
        is_error: false,
    }
}

/// Stateless executor for the `answer` tool. Returns placeholder text.
/// The actual Answer node + edges are created as a side-effect in the task handler.
pub fn execute_answer(question_id: &Uuid) -> ToolExecutionResult {
    ToolExecutionResult {
        content: ToolResultContent::text(format!("Answer submitted for {question_id}")),
        is_error: false,
    }
}
