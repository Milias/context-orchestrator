use crate::graph::tool_types::ToolCallArguments;
use crate::tasks::TaskMessage;
use tokio::sync::mpsc;
use uuid::Uuid;

pub struct ToolExecutionResult {
    pub content: String,
    pub is_error: bool,
}

/// Execute a tool call. Currently all tools are stubs that return errors.
/// Real implementations will be added as tools are built out.
// Stubs are sync now, but real implementations will need async for I/O.
#[allow(clippy::unused_async)]
pub async fn execute_tool(arguments: &ToolCallArguments) -> ToolExecutionResult {
    match arguments {
        ToolCallArguments::Plan { .. } => ToolExecutionResult {
            content: "Plan tool execution not yet implemented".to_string(),
            is_error: true,
        },
        ToolCallArguments::ReadFile { path } => ToolExecutionResult {
            content: format!("read_file not yet implemented (path: {path})"),
            is_error: true,
        },
        ToolCallArguments::WriteFile { path, .. } => ToolExecutionResult {
            content: format!("write_file not yet implemented (path: {path})"),
            is_error: true,
        },
        ToolCallArguments::WebSearch { query } => ToolExecutionResult {
            content: format!("web_search not yet implemented (query: {query})"),
            is_error: true,
        },
        ToolCallArguments::Unknown { tool_name, .. } => ToolExecutionResult {
            content: format!("Unknown tool: {tool_name}"),
            is_error: true,
        },
    }
}

/// Spawn a tokio task that executes a tool call and sends the result back via the channel.
pub fn spawn_tool_execution(
    tool_call_id: Uuid,
    arguments: ToolCallArguments,
    tx: mpsc::UnboundedSender<TaskMessage>,
) {
    tokio::spawn(async move {
        let result = execute_tool(&arguments).await;
        let _ = tx.send(TaskMessage::ToolCallCompleted {
            tool_call_id,
            content: result.content,
            is_error: result.is_error,
        });
    });
}

#[cfg(test)]
#[path = "tool_executor_tests.rs"]
mod tests;
