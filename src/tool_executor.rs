use crate::graph::tool_types::{ToolCallArguments, ToolResultContent};
use crate::llm::tool_types::{SchemaProperty, SchemaType, ToolDefinition, ToolInputSchema};
use crate::tasks::TaskMessage;
use tokio::sync::mpsc;
use uuid::Uuid;

/// Return the tool definitions to register with the LLM API.
pub fn registered_tool_definitions() -> Vec<ToolDefinition> {
    vec![ToolDefinition {
        name: "read_file".to_string(),
        description: "Read the contents of a file at the given path".to_string(),
        input_schema: ToolInputSchema {
            properties: vec![SchemaProperty {
                name: "path".to_string(),
                property_type: SchemaType::String,
                description: "Absolute or relative path to the file".to_string(),
                required: true,
            }],
        },
    }]
}

pub struct ToolExecutionResult {
    pub content: ToolResultContent,
    pub is_error: bool,
}

const MAX_READ_FILE_BYTES: usize = 100_000;

/// Execute a tool call and return the result.
pub async fn execute_tool(arguments: &ToolCallArguments) -> ToolExecutionResult {
    match arguments {
        ToolCallArguments::Plan { .. } => ToolExecutionResult {
            content: ToolResultContent::text("Plan tool execution not yet implemented"),
            is_error: true,
        },
        ToolCallArguments::ReadFile { path } => {
            let Ok(cwd) = std::env::current_dir() else {
                return ToolExecutionResult {
                    content: ToolResultContent::text("Error: could not determine working directory"),
                    is_error: true,
                };
            };
            let requested = if std::path::Path::new(path).is_absolute() {
                std::path::PathBuf::from(path)
            } else {
                cwd.join(path)
            };
            let canonical = match tokio::fs::canonicalize(&requested).await {
                Ok(p) => p,
                Err(e) => {
                    return ToolExecutionResult {
                        content: ToolResultContent::text(format!("Error reading file: {e}")),
                        is_error: true,
                    }
                }
            };
            let Ok(canonical_cwd) = tokio::fs::canonicalize(&cwd).await else {
                return ToolExecutionResult {
                    content: ToolResultContent::text("Error: could not resolve working directory"),
                    is_error: true,
                };
            };
            if !canonical.starts_with(&canonical_cwd) {
                return ToolExecutionResult {
                    content: ToolResultContent::text(format!(
                        "Error: path escapes working directory: {path}"
                    )),
                    is_error: true,
                };
            }
            match tokio::fs::read_to_string(&canonical).await {
                Ok(contents) => {
                    if contents.len() > MAX_READ_FILE_BYTES {
                        let mut boundary = MAX_READ_FILE_BYTES;
                        while boundary > 0 && !contents.is_char_boundary(boundary) {
                            boundary -= 1;
                        }
                        ToolExecutionResult {
                            content: ToolResultContent::text(format!(
                                "{}\n\n[truncated, {} bytes total]",
                                &contents[..boundary],
                                contents.len()
                            )),
                            is_error: false,
                        }
                    } else {
                        ToolExecutionResult {
                            content: ToolResultContent::text(contents),
                            is_error: false,
                        }
                    }
                }
                Err(e) => ToolExecutionResult {
                    content: ToolResultContent::text(format!("Error reading file: {e}")),
                    is_error: true,
                },
            }
        }
        ToolCallArguments::WriteFile { path, .. } => ToolExecutionResult {
            content: ToolResultContent::text(format!("write_file not yet implemented (path: {path})")),
            is_error: true,
        },
        ToolCallArguments::WebSearch { query } => ToolExecutionResult {
            content: ToolResultContent::text(format!(
                "web_search not yet implemented (query: {query})"
            )),
            is_error: true,
        },
        ToolCallArguments::Unknown { tool_name, .. } => ToolExecutionResult {
            content: ToolResultContent::text(format!(
                "Unrecognized tool or invalid arguments: {tool_name}"
            )),
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
