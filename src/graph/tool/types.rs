use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::super::WorkItemStatus;

// Re-export from result.rs so existing `tool_types::ToolResultContent` imports keep working.
pub use super::result::ToolResultContent;

// ── ToolName ─────────────────────────────────────────────────────────

/// Canonical identifier for a registered tool. Single source of truth for
/// tool name strings and their serde tag discriminants.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ToolName {
    Plan,
    AddTask,
    UpdateWorkItem,
    AddDependency,
    ReadFile,
    WriteFile,
    ListDirectory,
    SearchFiles,
    WebSearch,
    Set,
    Ask,
}

impl ToolName {
    // Wire names (single source of truth for string literals).
    const PLAN: &str = "plan";
    const ADD_TASK: &str = "add_task";
    const UPDATE_WORK_ITEM: &str = "update_work_item";
    const ADD_DEPENDENCY: &str = "add_dependency";
    const READ_FILE: &str = "read_file";
    const WRITE_FILE: &str = "write_file";
    const LIST_DIRECTORY: &str = "list_directory";
    const SEARCH_FILES: &str = "search_files";
    const WEB_SEARCH: &str = "web_search";
    const SET: &str = "set";
    const ASK: &str = "ask";

    /// Wire name as it appears in the API, triggers, and registry.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Plan => Self::PLAN,
            Self::AddTask => Self::ADD_TASK,
            Self::UpdateWorkItem => Self::UPDATE_WORK_ITEM,
            Self::AddDependency => Self::ADD_DEPENDENCY,
            Self::ReadFile => Self::READ_FILE,
            Self::WriteFile => Self::WRITE_FILE,
            Self::ListDirectory => Self::LIST_DIRECTORY,
            Self::SearchFiles => Self::SEARCH_FILES,
            Self::WebSearch => Self::WEB_SEARCH,
            Self::Set => Self::SET,
            Self::Ask => Self::ASK,
        }
    }

    /// Serde tag value matching the `#[serde(tag = "tool_type")]` discriminant
    /// on `ToolCallArguments`. Used by `parse_tool_arguments` to inject the tag.
    pub const fn serde_tag(self) -> &'static str {
        match self {
            Self::Plan => "Plan",
            Self::AddTask => "AddTask",
            Self::UpdateWorkItem => "UpdateWorkItem",
            Self::AddDependency => "AddDependency",
            Self::ReadFile => "ReadFile",
            Self::WriteFile => "WriteFile",
            Self::ListDirectory => "ListDirectory",
            Self::SearchFiles => "SearchFiles",
            Self::WebSearch => "WebSearch",
            Self::Set => "Set",
            Self::Ask => "Ask",
        }
    }

    /// Parse a wire name into a `ToolName`, if recognized.
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            Self::PLAN => Some(Self::Plan),
            Self::ADD_TASK => Some(Self::AddTask),
            Self::UPDATE_WORK_ITEM => Some(Self::UpdateWorkItem),
            Self::ADD_DEPENDENCY => Some(Self::AddDependency),
            Self::READ_FILE => Some(Self::ReadFile),
            Self::WRITE_FILE => Some(Self::WriteFile),
            Self::LIST_DIRECTORY => Some(Self::ListDirectory),
            Self::SEARCH_FILES => Some(Self::SearchFiles),
            Self::WEB_SEARCH => Some(Self::WebSearch),
            Self::SET => Some(Self::Set),
            Self::ASK => Some(Self::Ask),
            _ => None,
        }
    }
}

impl std::fmt::Display for ToolName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

// ── ToolCallStatus ───────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ToolCallStatus {
    Pending,
    Running,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "tool_type")]
pub enum ToolCallArguments {
    Plan {
        title: String,
        description: Option<String>,
    },
    ReadFile {
        path: String,
    },
    WriteFile {
        path: String,
        content: String,
    },
    ListDirectory {
        path: String,
        recursive: Option<bool>,
    },
    SearchFiles {
        pattern: String,
        path: Option<String>,
    },
    WebSearch {
        query: String,
    },
    /// Add a task under a plan or another task.
    AddTask {
        parent_id: Uuid,
        title: String,
        description: Option<String>,
    },
    /// Update a work item's status or description.
    UpdateWorkItem {
        id: Uuid,
        status: Option<WorkItemStatus>,
        description: Option<String>,
    },
    /// Declare a plan-to-plan dependency: `from_id` depends on `to_id`.
    AddDependency {
        from_id: Uuid,
        to_id: Uuid,
    },
    Set {
        key: String,
        value: String,
    },
    /// Ask a question routed to a backend (user, LLM, or auto).
    Ask {
        question: String,
        destination: super::super::node::QuestionDestination,
        about_node_id: Option<Uuid>,
        requires_approval: Option<bool>,
    },
    Unknown {
        tool_name: String,
        raw_json: String,
    },
}

impl ToolCallArguments {
    /// The tool's wire name. For typed variants this is derived from `ToolName`;
    /// for `Unknown` it returns the raw string received from the LLM.
    pub fn tool_name(&self) -> &str {
        match self {
            Self::Plan { .. } => ToolName::Plan.as_str(),
            Self::AddTask { .. } => ToolName::AddTask.as_str(),
            Self::UpdateWorkItem { .. } => ToolName::UpdateWorkItem.as_str(),
            Self::AddDependency { .. } => ToolName::AddDependency.as_str(),
            Self::ReadFile { .. } => ToolName::ReadFile.as_str(),
            Self::WriteFile { .. } => ToolName::WriteFile.as_str(),
            Self::ListDirectory { .. } => ToolName::ListDirectory.as_str(),
            Self::SearchFiles { .. } => ToolName::SearchFiles.as_str(),
            Self::WebSearch { .. } => ToolName::WebSearch.as_str(),
            Self::Set { .. } => ToolName::Set.as_str(),
            Self::Ask { .. } => ToolName::Ask.as_str(),
            Self::Unknown { tool_name, .. } => tool_name,
        }
    }

    /// One-line summary for display in the conversation view,
    /// e.g. `"read_file: src/main.rs"`.
    pub fn display_summary(&self) -> String {
        match self {
            Self::Plan { title, .. } => format!("plan: {title}"),
            Self::AddTask {
                title, parent_id, ..
            } => {
                format!("add_task: {title} (under {parent_id})")
            }
            Self::UpdateWorkItem { id, status, .. } => match status {
                Some(s) => format!("update_work_item: {id} → {s:?}"),
                None => format!("update_work_item: {id}"),
            },
            Self::AddDependency { from_id, to_id } => {
                format!("add_dependency: {from_id} → {to_id}")
            }
            Self::ReadFile { path } => format!("read_file: {path}"),
            Self::WriteFile { path, .. } => format!("write_file: {path}"),
            Self::ListDirectory { path, .. } => format!("list_directory: {path}"),
            Self::SearchFiles { pattern, path } => match path {
                Some(p) => format!("search_files: {pattern} in {p}"),
                None => format!("search_files: {pattern}"),
            },
            Self::WebSearch { query } => format!("web_search: {query}"),
            Self::Set { key, value } => format!("set: {key}={value}"),
            Self::Ask {
                question,
                destination,
                ..
            } => format!("ask ({destination:?}): {question}"),
            Self::Unknown {
                tool_name,
                raw_json,
            } => {
                let truncated: String = raw_json.chars().take(80).collect();
                if raw_json.len() > 80 {
                    format!("{tool_name}: {truncated}...")
                } else {
                    format!("{tool_name}: {truncated}")
                }
            }
        }
    }

    /// Serialize the tool's input fields as a raw JSON string (without the
    /// `#[serde(tag)]` discriminant). Used to reconstruct `ToolUse` content
    /// blocks for the Anthropic API.
    ///
    /// Note: `serde_json::Value` is used transiently to strip the tag field
    /// (same pattern as `RawJson`). It never appears in a struct field.
    ///
    /// Assumption: no typed variant has a field literally named `tool_type`.
    /// The `Unknown` variant bypasses stripping entirely (passes raw JSON through).
    pub fn to_input_json(&self) -> String {
        match self {
            Self::Unknown { raw_json, .. } => {
                // Validate that raw_json is actual JSON; fall back to empty object
                // to prevent RawJson::serialize from failing on the API request.
                if serde_json::from_str::<serde_json::Value>(raw_json).is_ok() {
                    raw_json.clone()
                } else {
                    "{}".to_string()
                }
            }
            other => {
                if let Ok(serde_json::Value::Object(mut map)) = serde_json::to_value(other) {
                    map.remove("tool_type");
                    serde_json::Value::Object(map).to_string()
                } else {
                    "{}".to_string()
                }
            }
        }
    }
}

// Tool result content types are in `tool_result.rs`.

/// Parse raw JSON from an LLM `tool_use` response into a typed `ToolCallArguments`.
///
/// LLM responses send tool input as raw field JSON (e.g. `{"path": "/foo"}`) without
/// the `#[serde(tag = "tool_type")]` discriminant that `ToolCallArguments` requires.
/// This function injects the tag before deserializing.
///
/// Note: `serde_json::Value` is used transiently to inject the tag field
/// (same pattern as `to_input_json`). It never appears in a struct field.
pub fn parse_tool_arguments(name: &str, raw_json: &str) -> ToolCallArguments {
    let Some(tool) = ToolName::from_str(name) else {
        return ToolCallArguments::Unknown {
            tool_name: name.to_string(),
            raw_json: raw_json.to_string(),
        };
    };
    let tag = tool.serde_tag();
    if let Ok(serde_json::Value::Object(mut map)) = serde_json::from_str(raw_json) {
        map.insert(
            "tool_type".to_string(),
            serde_json::Value::String(tag.to_string()),
        );
        if let Ok(parsed) = serde_json::from_value(serde_json::Value::Object(map)) {
            return parsed;
        }
    }
    ToolCallArguments::Unknown {
        tool_name: name.to_string(),
        raw_json: raw_json.to_string(),
    }
}
