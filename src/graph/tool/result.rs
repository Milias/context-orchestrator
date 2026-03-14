//! Tool result content types for the Anthropic API.
//!
//! Extracted from `tool_types.rs` to keep that module under the 400-line limit.
//! These types model the `tool_result.content` field format: either a plain
//! string or an array of typed content blocks (text + images).

use serde::{Deserialize, Serialize};

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

    // User: acceptable use of #[cfg(test)].
    #[cfg(test)]
    pub fn has_images(&self) -> bool {
        matches!(self, Self::Blocks(blocks) if blocks.iter().any(
            |b| matches!(b, ToolResultContentBlock::Image { .. })
        ))
    }
}
