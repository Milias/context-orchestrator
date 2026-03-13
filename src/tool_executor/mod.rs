mod list_directory;
mod read_file;
mod search_files;
mod security;
mod write_file;

use crate::graph::tool_types::{ToolCallArguments, ToolResultContent};

/// Directories to skip during recursive traversal.
const SKIP_DIRS: &[&str] = &[
    ".git",
    ".hg",
    ".svn",
    "target",
    "node_modules",
    "__pycache__",
    ".mypy_cache",
    "dist",
    "build",
];
use crate::llm::tool_types::{SchemaProperty, SchemaType, ToolDefinition, ToolInputSchema};
use crate::tasks::TaskMessage;
use tokio::sync::mpsc;
use uuid::Uuid;

pub struct ToolExecutionResult {
    pub content: ToolResultContent,
    pub is_error: bool,
}

/// Return the tool definitions to register with the LLM API.
pub fn registered_tool_definitions() -> Vec<ToolDefinition> {
    vec![
        ToolDefinition {
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
        },
        ToolDefinition {
            name: "write_file".to_string(),
            description: "Write content to a file, creating parent directories if needed. Overwrites existing files.".to_string(),
            input_schema: ToolInputSchema {
                properties: vec![
                    SchemaProperty {
                        name: "path".to_string(),
                        property_type: SchemaType::String,
                        description: "Absolute or relative path to the file".to_string(),
                        required: true,
                    },
                    SchemaProperty {
                        name: "content".to_string(),
                        property_type: SchemaType::String,
                        description: "The full content to write to the file".to_string(),
                        required: true,
                    },
                ],
            },
        },
        ToolDefinition {
            name: "list_directory".to_string(),
            description: "List files and directories at a given path".to_string(),
            input_schema: ToolInputSchema {
                properties: vec![
                    SchemaProperty {
                        name: "path".to_string(),
                        property_type: SchemaType::String,
                        description: "Path to the directory to list".to_string(),
                        required: true,
                    },
                    SchemaProperty {
                        name: "recursive".to_string(),
                        property_type: SchemaType::Boolean,
                        description: "If true, list recursively. Defaults to false."
                            .to_string(),
                        required: false,
                    },
                ],
            },
        },
        ToolDefinition {
            name: "search_files".to_string(),
            description: "Search for a regex pattern across files in the project"
                .to_string(),
            input_schema: ToolInputSchema {
                properties: vec![
                    SchemaProperty {
                        name: "pattern".to_string(),
                        property_type: SchemaType::String,
                        description: "Regex pattern to search for".to_string(),
                        required: true,
                    },
                    SchemaProperty {
                        name: "path".to_string(),
                        property_type: SchemaType::String,
                        description: "Directory to search in. Defaults to the current working directory.".to_string(),
                        required: false,
                    },
                ],
            },
        },
    ]
}

/// Execute a tool call and return the result.
pub async fn execute_tool(arguments: &ToolCallArguments) -> ToolExecutionResult {
    match arguments {
        ToolCallArguments::Plan { .. } => ToolExecutionResult {
            content: ToolResultContent::text("Plan tool execution not yet implemented"),
            is_error: true,
        },
        ToolCallArguments::ReadFile { path } => read_file::execute(path).await,
        ToolCallArguments::WriteFile { path, content } => write_file::execute(path, content).await,
        ToolCallArguments::ListDirectory { path, recursive } => {
            list_directory::execute(path, recursive.unwrap_or(false)).await
        }
        ToolCallArguments::SearchFiles { pattern, path } => {
            search_files::execute(pattern, path.as_deref()).await
        }
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
mod tests;
