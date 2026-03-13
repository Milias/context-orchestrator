use serde::{Deserialize, Serialize};

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
        raw_input: String,
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
    Unknown {
        tool_name: String,
        raw_json: String,
    },
}

impl ToolCallArguments {
    pub fn tool_name(&self) -> &str {
        match self {
            Self::Plan { .. } => "plan",
            Self::ReadFile { .. } => "read_file",
            Self::WriteFile { .. } => "write_file",
            Self::ListDirectory { .. } => "list_directory",
            Self::SearchFiles { .. } => "search_files",
            Self::WebSearch { .. } => "web_search",
            Self::Unknown { tool_name, .. } => tool_name,
        }
    }

    /// One-line summary for display in the conversation view,
    /// e.g. `"read_file: src/main.rs"`.
    pub fn display_summary(&self) -> String {
        match self {
            Self::Plan { description, .. } => match description {
                Some(d) => format!("plan: {d}"),
                None => "plan".to_string(),
            },
            Self::ReadFile { path } => format!("read_file: {path}"),
            Self::WriteFile { path, .. } => format!("write_file: {path}"),
            Self::ListDirectory { path, .. } => format!("list_directory: {path}"),
            Self::SearchFiles { pattern, path } => match path {
                Some(p) => format!("search_files: {pattern} in {p}"),
                None => format!("search_files: {pattern}"),
            },
            Self::WebSearch { query } => format!("web_search: {query}"),
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
    let tag = match name {
        "plan" => "Plan",
        "read_file" => "ReadFile",
        "write_file" => "WriteFile",
        "list_directory" => "ListDirectory",
        "search_files" => "SearchFiles",
        "web_search" => "WebSearch",
        _ => {
            return ToolCallArguments::Unknown {
                tool_name: name.to_string(),
                raw_json: raw_json.to_string(),
            }
        }
    };
    if let Ok(serde_json::Value::Object(mut map)) = serde_json::from_str(raw_json) {
        map.insert(
            "tool_type".to_string(),
            serde_json::Value::String(tag.to_string()),
        );
        if let Ok(parsed) = serde_json::from_value(serde_json::Value::Object(map)) {
            return parsed;
        }
    }
    // Plan preserves raw input as fallback; others become Unknown.
    if name == "plan" {
        ToolCallArguments::Plan {
            raw_input: raw_json.to_string(),
            description: None,
        }
    } else {
        ToolCallArguments::Unknown {
            tool_name: name.to_string(),
            raw_json: raw_json.to_string(),
        }
    }
}
