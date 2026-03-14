use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::WorkItemStatus;

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
}

impl ToolName {
    /// Wire name as it appears in the API, triggers, and registry.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Plan => "plan",
            Self::AddTask => "add_task",
            Self::UpdateWorkItem => "update_work_item",
            Self::AddDependency => "add_dependency",
            Self::ReadFile => "read_file",
            Self::WriteFile => "write_file",
            Self::ListDirectory => "list_directory",
            Self::SearchFiles => "search_files",
            Self::WebSearch => "web_search",
            Self::Set => "set",
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
        }
    }

    /// Parse a wire name into a `ToolName`, if recognized.
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "plan" => Some(Self::Plan),
            "add_task" => Some(Self::AddTask),
            "update_work_item" => Some(Self::UpdateWorkItem),
            "add_dependency" => Some(Self::AddDependency),
            "read_file" => Some(Self::ReadFile),
            "write_file" => Some(Self::WriteFile),
            "list_directory" => Some(Self::ListDirectory),
            "search_files" => Some(Self::SearchFiles),
            "web_search" => Some(Self::WebSearch),
            "set" => Some(Self::Set),
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

// ── Tool result content types ─────────────────────────────────────

/// A content block within a structured tool result.
///
/// Currently supports `text` and `image` block types. The Anthropic API also
/// defines `document`, `search_result`, and `tool_reference` types — these are
/// not modeled here. Deserializing an unsupported block type will fail; this is
/// acceptable because `ToolResultContent` is only constructed client-side and
/// never deserialized from API responses.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ToolResultContentBlock {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "image")]
    Image { source: ImageSource },
}

/// Source data for an image content block.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ImageSource {
    #[serde(rename = "base64")]
    Base64 { media_type: String, data: String },
}

/// Tool result content: plain string or array of content blocks (text + images).
/// Matches the Anthropic API `tool_result.content` format.
///
/// `#[serde(untagged)]` with `Text` first ensures existing V2 graphs with
/// `"content": "string"` deserialize correctly — no migration needed.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ToolResultContent {
    Text(String),
    Blocks(Vec<ToolResultContentBlock>),
}

impl ToolResultContent {
    pub fn text(s: impl Into<String>) -> Self {
        Self::Text(s.into())
    }

    /// Returns the first text block as `&str`, or `""` if there are no text blocks.
    /// For `Blocks` with multiple `Text` entries, only the first is returned.
    pub fn text_content(&self) -> &str {
        match self {
            Self::Text(s) => s,
            Self::Blocks(blocks) => blocks
                .iter()
                .find_map(|b| match b {
                    ToolResultContentBlock::Text { text } => Some(text.as_str()),
                    ToolResultContentBlock::Image { .. } => None,
                })
                .unwrap_or(""),
        }
    }

    /// Approximate byte length for token budget calculations.
    /// For images, uses base64 data length as a rough proxy — actual API token
    /// cost is dimension-based, but this suffices for heuristic context truncation
    /// since precise counting is done via the `count_tokens` API endpoint.
    pub fn char_len(&self) -> usize {
        match self {
            Self::Text(s) => s.len(),
            Self::Blocks(blocks) => blocks
                .iter()
                .map(|b| match b {
                    ToolResultContentBlock::Text { text } => text.len(),
                    ToolResultContentBlock::Image { source } => match source {
                        ImageSource::Base64 { data, .. } => data.len(),
                    },
                })
                .sum(),
        }
    }

    // User: accepatble use of #[cfg(test)].
    #[cfg(test)]
    pub fn has_images(&self) -> bool {
        matches!(self, Self::Blocks(blocks) if blocks.iter().any(
            |b| matches!(b, ToolResultContentBlock::Image { .. })
        ))
    }
}

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
