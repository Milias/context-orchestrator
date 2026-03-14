//! Execution logic for plan-related tools: `plan`, `add_task`,
//! `update_work_item`, `add_dependency`.
//!
//! Graph mutations (`WorkItem` creation, `SubtaskOf` edges, status propagation)
//! happen as side-effects in `plan_effects.rs`.

use super::ToolExecutionResult;
use crate::graph::tool_types::ToolResultContent;

/// Execute the `plan` tool. Returns placeholder — the completion handler
/// creates the `Plan` `WorkItem` and enriches the result with the UUID.
pub fn execute_plan(title: &str) -> ToolExecutionResult {
    ToolExecutionResult {
        content: ToolResultContent::text(format!("Created plan: {title}")),
        is_error: false,
    }
}

/// Execute `add_task`. UUID validation is handled by serde (`Uuid` type).
/// Returns placeholder — completion handler creates the `WorkItem` + `SubtaskOf` edge.
pub fn execute_add_task(title: &str) -> ToolExecutionResult {
    ToolExecutionResult {
        content: ToolResultContent::text(format!("Added task: {title}")),
        is_error: false,
    }
}

/// Execute `update_work_item`. Returns placeholder — completion handler
/// applies the status change and propagates.
pub fn execute_update_work_item() -> ToolExecutionResult {
    ToolExecutionResult {
        content: ToolResultContent::text("Work item updated"),
        is_error: false,
    }
}

/// Execute `add_dependency`. Returns placeholder — completion handler
/// validates both plans exist and creates the `DependsOn` edge.
pub fn execute_add_dependency() -> ToolExecutionResult {
    ToolExecutionResult {
        content: ToolResultContent::text("Dependency added"),
        is_error: false,
    }
}
