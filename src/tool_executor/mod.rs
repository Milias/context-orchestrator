//! Tool registry, definitions, and execution spawning.
//!
//! The registry is the single source of truth for all tools. Discovery,
//! LLM registration, autocomplete, and trigger parsing all derive from it.
//! Execution logic lives in `execute.rs`; per-tool implementations in
//! their own submodules.

mod execute;
mod list_directory;
mod plan_tools;
mod qa_tools;
mod read_file;
mod search_files;
mod security;
mod write_file;

pub use execute::{apply_config_set, execute_tool, ConfigKey};

use crate::graph::tool_types::{ToolCallArguments, ToolName, ToolResultContent};
use crate::llm::tool_types::{SchemaProperty, SchemaType, ToolDefinition, ToolInputSchema};
use crate::tasks::TaskMessage;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

/// Result of executing a tool call.
pub struct ToolExecutionResult {
    pub content: ToolResultContent,
    pub is_error: bool,
}

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

// ── Tool Registry ───────────────────────────────────────────────────

/// Metadata for a registered tool. Every tool is equally callable by users
/// (via `/name args`) and by the LLM (via `tool_use`).
pub struct ToolRegistryEntry {
    pub name: ToolName,
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
    tools.extend(plan_tools());
    tools.extend(qa_tools());
    tools.extend(filesystem_tools());
    tools
}

/// Config tools.
fn config_tools() -> Vec<ToolRegistryEntry> {
    vec![entry(
        ToolName::Set,
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
    )]
}

/// Plan and task management tools.
fn plan_tools() -> Vec<ToolRegistryEntry> {
    vec![
        entry(
            ToolName::Plan,
            "Create a plan. Returns the plan UUID. Use add_task to decompose it into steps.",
            &[
                prop(
                    "title",
                    SchemaType::String,
                    "Concise title for the plan",
                    true,
                ),
                prop(
                    "description",
                    SchemaType::String,
                    "Detailed description of the plan",
                    false,
                ),
            ],
        ),
        entry(
            ToolName::AddTask,
            "Add a task under a plan or another task. Provide the parent's UUID.",
            &[
                prop(
                    "parent_id",
                    SchemaType::String,
                    "UUID of the parent plan or task",
                    true,
                ),
                prop("title", SchemaType::String, "Title of the task", true),
                prop(
                    "description",
                    SchemaType::String,
                    "Description of the task",
                    false,
                ),
            ],
        ),
        entry(
            ToolName::UpdateWorkItem,
            "Update a work item's status (todo/active/done) or description.",
            &[
                prop(
                    "id",
                    SchemaType::String,
                    "UUID of the work item to update",
                    true,
                ),
                prop(
                    "status",
                    SchemaType::String,
                    "New status: todo, active, or done",
                    false,
                ),
                prop(
                    "description",
                    SchemaType::String,
                    "Updated description",
                    false,
                ),
            ],
        ),
        entry(
            ToolName::AddDependency,
            "Declare that one plan depends on another completing first. from_id depends on to_id.",
            &[
                prop(
                    "from_id",
                    SchemaType::String,
                    "UUID of the plan that depends on another",
                    true,
                ),
                prop(
                    "to_id",
                    SchemaType::String,
                    "UUID of the prerequisite plan",
                    true,
                ),
            ],
        ),
    ]
}

/// Q/A tools: ask questions and provide answers.
fn qa_tools() -> Vec<ToolRegistryEntry> {
    vec![
        entry(
            ToolName::Ask,
            "Ask a question to the user, an LLM, or auto-route. Returns a question UUID. \
             The answer arrives asynchronously and resolves any DependsOn edges.",
            &[
                prop("question", SchemaType::String, "The question to ask", true),
                prop(
                    "destination",
                    SchemaType::String,
                    "Who answers: user, llm, or auto",
                    true,
                ),
                prop(
                    "about_node_id",
                    SchemaType::String,
                    "UUID of a node this question is about (optional)",
                    false,
                ),
                prop(
                    "requires_approval",
                    SchemaType::Boolean,
                    "If true, LLM answers require user approval before resolving (default: false)",
                    false,
                ),
            ],
        ),
        entry(
            ToolName::Answer,
            "Answer a pending question that has been claimed by you. \
             The question must be in Claimed status.",
            &[
                prop(
                    "question_id",
                    SchemaType::String,
                    "UUID of the question to answer",
                    true,
                ),
                prop("content", SchemaType::String, "The answer text", true),
            ],
        ),
    ]
}

/// Filesystem tools: read, write, list, search.
fn filesystem_tools() -> Vec<ToolRegistryEntry> {
    vec![
        entry(
            ToolName::ReadFile,
            "Read the contents of a file at the given path",
            &[prop(
                "path",
                SchemaType::String,
                "Absolute or relative path to the file",
                true,
            )],
        ),
        entry(
            ToolName::WriteFile,
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
            ToolName::ListDirectory,
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
            ToolName::SearchFiles,
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
fn entry(name: ToolName, description: &'static str, props: &[SchemaProperty]) -> ToolRegistryEntry {
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
            name: entry.name.as_str().to_string(),
            description: entry.description.to_string(),
            input_schema: entry.input_schema.clone(),
        })
        .collect()
}

// ── Spawn ───────────────────────────────────────────────────────────

/// Spawn a tokio task that executes a tool call and sends the result back via the channel.
/// The task is cancelled when `cancel_token` fires, sending a cancellation error.
///
/// `working_dir` scopes file operations to a specific directory (e.g., a git
/// worktree for task agents). When `None`, the process CWD is used.
pub fn spawn_tool_execution(
    tool_call_id: Uuid,
    arguments: ToolCallArguments,
    tx: mpsc::UnboundedSender<TaskMessage>,
    cancel_token: CancellationToken,
    working_dir: Option<std::path::PathBuf>,
) {
    tokio::spawn(async move {
        let (content, is_error) = tokio::select! {
            result = execute_tool(&arguments, working_dir.as_deref()) => (result.content, result.is_error),
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

#[cfg(test)]
#[path = "execute_tests.rs"]
mod execute_tests;

#[cfg(test)]
#[path = "plan_qa_tests.rs"]
mod plan_qa_tests;
