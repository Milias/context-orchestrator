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
    let mut tools = config_tools();
    tools.extend(filesystem_tools());
    tools
}

/// Config and planning tools.
fn config_tools() -> Vec<ToolRegistryEntry> {
    vec![
        entry(
            "set",
            "Set a runtime configuration value (e.g. max_tokens, model)",
            &[
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
            ],
        ),
        entry(
            "plan",
            "Create a structured work item from a description",
            &[
                prop(
                    "title",
                    SchemaType::String,
                    "Concise title for the work item",
                    true,
                ),
                prop(
                    "description",
                    SchemaType::String,
                    "Detailed description of the work item",
                    false,
                ),
            ],
        ),
    ]
}

/// Filesystem tools: read, write, list, search.
fn filesystem_tools() -> Vec<ToolRegistryEntry> {
    vec![
        entry(
            "read_file",
            "Read the contents of a file at the given path",
            &[prop(
                "path",
                SchemaType::String,
                "Absolute or relative path to the file",
                true,
            )],
        ),
        entry(
            "write_file",
            "Write content to a file, creating parent directories if needed",
            &[
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
            ],
        ),
        entry(
            "list_directory",
            "List files and directories at a given path",
            &[
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
            ],
        ),
        entry(
            "search_files",
            "Search for a regex pattern across files in the project",
            &[
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
            ],
        ),
    ]
}

/// Shorthand: build a `ToolRegistryEntry`.
fn entry(
    name: &'static str,
    description: &'static str,
    props: &[SchemaProperty],
) -> ToolRegistryEntry {
    ToolRegistryEntry {
        name,
        description,
        input_schema: schema(props),
    }
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

// ── ConfigKey ────────────────────────────────────────────────────────

/// Typed runtime configuration keys for the `set` tool.
/// String parsing happens at the validation boundary; downstream logic
/// matches on enum variants with exhaustiveness checking.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigKey {
    MaxTokens,
    MaxContextTokens,
    Model,
    MaxToolLoopIterations,
}

impl ConfigKey {
    /// All valid variants. Exhaustive match in `FromStr` ensures this stays in sync.
    const ALL: &[Self] = &[
        Self::MaxTokens,
        Self::MaxContextTokens,
        Self::Model,
        Self::MaxToolLoopIterations,
    ];

    /// All valid keys as a comma-separated string, for error messages.
    fn all_display() -> String {
        Self::ALL
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(", ")
    }
}

impl std::fmt::Display for ConfigKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MaxTokens => write!(f, "max_tokens"),
            Self::MaxContextTokens => write!(f, "max_context_tokens"),
            Self::Model => write!(f, "model"),
            Self::MaxToolLoopIterations => write!(f, "max_tool_loop_iterations"),
        }
    }
}

impl std::str::FromStr for ConfigKey {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "max_tokens" => Ok(Self::MaxTokens),
            "max_context_tokens" => Ok(Self::MaxContextTokens),
            "model" => Ok(Self::Model),
            "max_tool_loop_iterations" => Ok(Self::MaxToolLoopIterations),
            _ => Err(format!(
                "Unknown config key: {s}. Valid keys: {}",
                Self::all_display()
            )),
        }
    }
}

/// Execute a tool call and return the result.
pub async fn execute_tool(arguments: &ToolCallArguments) -> ToolExecutionResult {
    match arguments {
        ToolCallArguments::Set { key, value } => execute_set(key, value),
        ToolCallArguments::Plan { title, .. } => ToolExecutionResult {
            content: ToolResultContent::text(format!("Created work item: {title}")),
            is_error: false,
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

/// Validate a `set` command. Parses the string key into `ConfigKey` at the boundary.
/// The actual config mutation happens in the task handler (which has `&mut AppConfig`).
fn execute_set(key: &str, value: &str) -> ToolExecutionResult {
    if value.is_empty() {
        return set_err(format!("Missing value for {key}"));
    }
    let config_key = match key.parse::<ConfigKey>() {
        Ok(k) => k,
        Err(msg) => return set_err(msg),
    };
    if let Err(msg) = validate_set_value(config_key, value) {
        return set_err(msg);
    }
    ToolExecutionResult {
        content: ToolResultContent::text(format!("{config_key} set to {value}")),
        is_error: false,
    }
}

/// Validate value range for numeric config keys.
fn validate_set_value(key: ConfigKey, value: &str) -> Result<(), String> {
    match key {
        ConfigKey::MaxTokens => validate_u32_range(key, value, 1, 128_000),
        ConfigKey::MaxContextTokens => validate_u32_range(key, value, 1000, 1_000_000),
        ConfigKey::MaxToolLoopIterations => {
            let v = value
                .parse::<usize>()
                .map_err(|_| format!("Invalid value for {key}: expected a number"))?;
            if v == 0 || v > 100 {
                return Err(format!("{key} must be between 1 and 100"));
            }
            Ok(())
        }
        ConfigKey::Model => Ok(()), // accepts any non-empty string
    }
}

fn validate_u32_range(key: ConfigKey, value: &str, min: u32, max: u32) -> Result<(), String> {
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
pub fn apply_config_set(config: &mut crate::config::AppConfig, key: ConfigKey, value: &str) {
    match key {
        ConfigKey::MaxTokens => {
            if let Ok(v) = value.parse::<u32>() {
                config.max_tokens = v;
            }
        }
        ConfigKey::MaxContextTokens => {
            if let Ok(v) = value.parse::<u32>() {
                config.max_context_tokens = v;
            }
        }
        ConfigKey::Model => {
            config.anthropic_model = value.to_string();
        }
        ConfigKey::MaxToolLoopIterations => {
            if let Ok(v) = value.parse::<usize>() {
                config.max_tool_loop_iterations = v;
            }
        }
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
