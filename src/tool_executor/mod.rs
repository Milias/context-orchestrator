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
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

pub struct ToolExecutionResult {
    pub content: ToolResultContent,
    pub is_error: bool,
}

// ── Tool Registry ───────────────────────────────────────────────────

/// Metadata for a registered tool. Every tool is equally callable by users
/// (via `/name args`) and by the LLM (via `tool_use`).
pub struct ToolRegistryEntry {
    pub name: &'static str,
    pub description: &'static str,
    pub input_schema: ToolInputSchema,
}

/// Single source of truth for all tools. Discovery, LLM registration,
/// autocomplete, and trigger parsing all derive from this registry.
pub fn tool_registry() -> &'static [ToolRegistryEntry] {
    use std::sync::LazyLock;
    static REGISTRY: LazyLock<Vec<ToolRegistryEntry>> = LazyLock::new(build_registry);
    &REGISTRY
}

fn build_registry() -> Vec<ToolRegistryEntry> {
    vec![
        ToolRegistryEntry {
            name: "set",
            description: "Set a runtime configuration value (e.g. max_tokens, model)",
            input_schema: schema(&[
                prop(
                    "key",
                    SchemaType::String,
                    "Config key: max_tokens, max_context_tokens, model, max_tool_loop_iterations",
                    true,
                ),
                prop(
                    "value",
                    SchemaType::String,
                    "New value for the config key",
                    true,
                ),
            ]),
        },
        ToolRegistryEntry {
            name: "plan",
            description: "Create a structured work item from a description",
            input_schema: schema(&[prop(
                "raw_input",
                SchemaType::String,
                "Free-text description of the work item",
                true,
            )]),
        },
        ToolRegistryEntry {
            name: "read_file",
            description: "Read the contents of a file at the given path",
            input_schema: schema(&[prop(
                "path",
                SchemaType::String,
                "Absolute or relative path to the file",
                true,
            )]),
        },
        ToolRegistryEntry {
            name: "write_file",
            description: "Write content to a file, creating parent directories if needed",
            input_schema: schema(&[
                prop(
                    "path",
                    SchemaType::String,
                    "Absolute or relative path to the file",
                    true,
                ),
                prop(
                    "content",
                    SchemaType::String,
                    "The full content to write to the file",
                    true,
                ),
            ]),
        },
        ToolRegistryEntry {
            name: "list_directory",
            description: "List files and directories at a given path",
            input_schema: schema(&[
                prop(
                    "path",
                    SchemaType::String,
                    "Path to the directory to list",
                    true,
                ),
                prop(
                    "recursive",
                    SchemaType::Boolean,
                    "If true, list recursively. Defaults to false.",
                    false,
                ),
            ]),
        },
        ToolRegistryEntry {
            name: "search_files",
            description: "Search for a regex pattern across files in the project",
            input_schema: schema(&[
                prop(
                    "pattern",
                    SchemaType::String,
                    "Regex pattern to search for",
                    true,
                ),
                prop(
                    "path",
                    SchemaType::String,
                    "Directory to search in. Defaults to cwd.",
                    false,
                ),
            ]),
        },
    ]
}

/// Shorthand: build a `SchemaProperty`.
fn prop(name: &str, ty: SchemaType, desc: &str, required: bool) -> SchemaProperty {
    SchemaProperty {
        name: name.to_string(),
        property_type: ty,
        description: desc.to_string(),
        required,
    }
}

/// Shorthand: build a `ToolInputSchema` from a slice of properties.
fn schema(props: &[SchemaProperty]) -> ToolInputSchema {
    ToolInputSchema {
        properties: props.to_vec(),
    }
}

/// Return tool definitions for the LLM API, derived from the registry.
pub fn registered_tool_definitions() -> Vec<ToolDefinition> {
    tool_registry()
        .iter()
        .map(|entry| ToolDefinition {
            name: entry.name.to_string(),
            description: entry.description.to_string(),
            input_schema: entry.input_schema.clone(),
        })
        .collect()
}

/// Known config keys for the `set` tool.
const VALID_SET_KEYS: &[&str] = &[
    "max_tokens",
    "max_context_tokens",
    "model",
    "max_tool_loop_iterations",
];

/// Execute a tool call and return the result.
pub async fn execute_tool(arguments: &ToolCallArguments) -> ToolExecutionResult {
    match arguments {
        ToolCallArguments::Set { key, value } => execute_set(key, value),
        ToolCallArguments::Plan { .. } => ToolExecutionResult {
            content: ToolResultContent::text(
                "Plan tool: use /plan <description> to create work items",
            ),
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

/// Validate a `set` command. The actual config mutation happens in the task handler
/// (which has access to `&mut AppConfig`), not here.
fn execute_set(key: &str, value: &str) -> ToolExecutionResult {
    if value.is_empty() {
        return set_err(format!("Missing value for {key}"));
    }
    if !VALID_SET_KEYS.contains(&key) {
        return set_err(format!(
            "Unknown config key: {key}. Valid keys: {}",
            VALID_SET_KEYS.join(", ")
        ));
    }
    if let Err(msg) = validate_set_value(key, value) {
        return set_err(msg);
    }
    ToolExecutionResult {
        content: ToolResultContent::text(format!("{key} set to {value}")),
        is_error: false,
    }
}

/// Validate value range for numeric config keys.
fn validate_set_value(key: &str, value: &str) -> Result<(), String> {
    match key {
        "max_tokens" => validate_u32_range(key, value, 1, 128_000),
        "max_context_tokens" => validate_u32_range(key, value, 1000, 1_000_000),
        "max_tool_loop_iterations" => {
            let v = value
                .parse::<usize>()
                .map_err(|_| format!("Invalid value for {key}: expected a number"))?;
            if v == 0 || v > 100 {
                return Err(format!("{key} must be between 1 and 100"));
            }
            Ok(())
        }
        _ => Ok(()), // "model" accepts any non-empty string
    }
}

fn validate_u32_range(key: &str, value: &str, min: u32, max: u32) -> Result<(), String> {
    let v = value
        .parse::<u32>()
        .map_err(|_| format!("Invalid value for {key}: expected a number"))?;
    if v < min || v > max {
        return Err(format!("{key} must be between {min} and {max}"));
    }
    Ok(())
}

fn set_err(msg: String) -> ToolExecutionResult {
    ToolExecutionResult {
        content: ToolResultContent::text(msg),
        is_error: true,
    }
}

/// Apply a `set` config mutation. Called from the main event loop where `AppConfig` is mutable.
/// Session-scoped — does not persist across restarts.
pub fn apply_config_set(config: &mut crate::config::AppConfig, key: &str, value: &str) {
    match key {
        "max_tokens" => {
            if let Ok(v) = value.parse::<u32>() {
                config.max_tokens = v;
            }
        }
        "max_context_tokens" => {
            if let Ok(v) = value.parse::<u32>() {
                config.max_context_tokens = v;
            }
        }
        "model" => {
            config.anthropic_model = value.to_string();
        }
        "max_tool_loop_iterations" => {
            if let Ok(v) = value.parse::<usize>() {
                config.max_tool_loop_iterations = v;
            }
        }
        _ => {}
    }
}

/// Spawn a tokio task that executes a tool call and sends the result back via the channel.
/// The task is cancelled when `cancel_token` fires, sending a cancellation error.
pub fn spawn_tool_execution(
    tool_call_id: Uuid,
    arguments: ToolCallArguments,
    tx: mpsc::UnboundedSender<TaskMessage>,
    cancel_token: CancellationToken,
) {
    tokio::spawn(async move {
        let (content, is_error) = tokio::select! {
            result = execute_tool(&arguments) => (result.content, result.is_error),
            () = cancel_token.cancelled() => {
                (ToolResultContent::text("Tool execution cancelled"), true)
            }
        };
        let _ = tx.send(TaskMessage::ToolCallCompleted {
            tool_call_id,
            content,
            is_error,
        });
    });
}

#[cfg(test)]
mod tests;
