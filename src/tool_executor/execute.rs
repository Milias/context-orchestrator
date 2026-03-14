//! Tool execution dispatch and per-tool executors.
//!
//! `execute_tool()` matches `ToolCallArguments` variants and delegates to
//! per-tool execution functions. Side-effects (graph mutations, config changes)
//! happen in the task handler, not here — execution is stateless.

use super::list_directory;
use super::plan_tools;
use super::qa_tools;
use super::read_file;
use super::search_files;
use super::write_file;
use super::ToolExecutionResult;

use crate::graph::tool_types::{ToolCallArguments, ToolResultContent};

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

// ── Tool Execution ──────────────────────────────────────────────────

/// Execute a tool call and return the result.
pub async fn execute_tool(arguments: &ToolCallArguments) -> ToolExecutionResult {
    match arguments {
        ToolCallArguments::Set { key, value } => execute_set(key, value),
        ToolCallArguments::Plan { title, .. } => plan_tools::execute_plan(title),
        ToolCallArguments::AddTask { title, .. } => plan_tools::execute_add_task(title),
        ToolCallArguments::UpdateWorkItem { .. } => plan_tools::execute_update_work_item(),
        ToolCallArguments::AddDependency { .. } => plan_tools::execute_add_dependency(),
        ToolCallArguments::ReadFile { path } => read_file::execute(path).await,
        ToolCallArguments::WriteFile { path, content } => write_file::execute(path, content).await,
        ToolCallArguments::ListDirectory { path, recursive } => {
            list_directory::execute(path, recursive.unwrap_or(false)).await
        }
        ToolCallArguments::SearchFiles { pattern, path } => {
            search_files::execute(pattern, path.as_deref()).await
        }
        ToolCallArguments::Ask { question, .. } => qa_tools::execute_ask(question),
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
