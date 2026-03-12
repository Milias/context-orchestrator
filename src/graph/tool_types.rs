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
            Self::WebSearch { .. } => "web_search",
            Self::Unknown { tool_name, .. } => tool_name,
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

    /// First text content as `&str`, or `""` for image-only results.
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
/// Dispatches on tool name, attempts typed deserialization, falls back to `Unknown`.
pub fn parse_tool_arguments(name: &str, raw_json: &str) -> ToolCallArguments {
    match name {
        "plan" => serde_json::from_str(raw_json).unwrap_or_else(|_| ToolCallArguments::Plan {
            raw_input: raw_json.to_string(),
            description: None,
        }),
        "read_file" => {
            serde_json::from_str(raw_json).unwrap_or_else(|_| ToolCallArguments::Unknown {
                tool_name: name.to_string(),
                raw_json: raw_json.to_string(),
            })
        }
        "write_file" => {
            serde_json::from_str(raw_json).unwrap_or_else(|_| ToolCallArguments::Unknown {
                tool_name: name.to_string(),
                raw_json: raw_json.to_string(),
            })
        }
        "web_search" => {
            serde_json::from_str(raw_json).unwrap_or_else(|_| ToolCallArguments::Unknown {
                tool_name: name.to_string(),
                raw_json: raw_json.to_string(),
            })
        }
        _ => ToolCallArguments::Unknown {
            tool_name: name.to_string(),
            raw_json: raw_json.to_string(),
        },
    }
}
