use serde::{Deserialize, Serialize};

/// A tool definition that can be sent to the LLM API.
#[derive(Debug, Clone)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub input_schema: ToolInputSchema,
}

/// Schema describing the expected input properties for a tool.
#[derive(Debug, Clone)]
pub struct ToolInputSchema {
    pub properties: Vec<SchemaProperty>,
}

/// A single property within a tool's input schema.
#[derive(Debug, Clone)]
pub struct SchemaProperty {
    pub name: String,
    pub property_type: SchemaType,
    pub description: String,
    pub required: bool,
}

/// JSON Schema types supported in tool definitions.
/// All JSON Schema primitives are represented for completeness; only `String`
/// is used so far but the rest are needed as tool definitions expand.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub enum SchemaType {
    String,
    Integer,
    Number,
    Boolean,
    Array { item_type: Box<SchemaType> },
    Object,
}

// ── Anthropic API serialization structs ─────────────────────────────

/// Wire format for a tool definition sent to the Anthropic API.
#[derive(Serialize)]
pub(crate) struct ApiToolDefinition {
    pub name: String,
    pub description: String,
    pub input_schema: ApiInputSchema,
}

/// Wire format for the `input_schema` field.
#[derive(Serialize)]
pub(crate) struct ApiInputSchema {
    #[serde(rename = "type")]
    pub schema_type: String,
    pub properties: std::collections::HashMap<String, ApiPropertySchema>,
    pub required: Vec<String>,
}

/// Wire format for a single property in the schema.
#[derive(Serialize)]
pub(crate) struct ApiPropertySchema {
    #[serde(rename = "type")]
    pub property_type: String,
    pub description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub items: Option<Box<ApiPropertySchema>>,
}

impl ToolDefinition {
    /// Convert to the Anthropic API wire format.
    pub(crate) fn to_api(&self) -> ApiToolDefinition {
        let mut properties = std::collections::HashMap::new();
        let mut required = Vec::new();

        for prop in &self.input_schema.properties {
            properties.insert(
                prop.name.clone(),
                schema_type_to_api(&prop.property_type, &prop.description),
            );
            if prop.required {
                required.push(prop.name.clone());
            }
        }

        ApiToolDefinition {
            name: self.name.clone(),
            description: self.description.clone(),
            input_schema: ApiInputSchema {
                schema_type: "object".to_string(),
                properties,
                required,
            },
        }
    }
}

fn schema_type_to_api(schema_type: &SchemaType, description: &str) -> ApiPropertySchema {
    match schema_type {
        SchemaType::String => ApiPropertySchema {
            property_type: "string".to_string(),
            description: description.to_string(),
            items: None,
        },
        SchemaType::Integer => ApiPropertySchema {
            property_type: "integer".to_string(),
            description: description.to_string(),
            items: None,
        },
        SchemaType::Number => ApiPropertySchema {
            property_type: "number".to_string(),
            description: description.to_string(),
            items: None,
        },
        SchemaType::Boolean => ApiPropertySchema {
            property_type: "boolean".to_string(),
            description: description.to_string(),
            items: None,
        },
        SchemaType::Array { item_type } => ApiPropertySchema {
            property_type: "array".to_string(),
            description: description.to_string(),
            items: Some(Box::new(schema_type_to_api(item_type, ""))),
        },
        SchemaType::Object => ApiPropertySchema {
            property_type: "object".to_string(),
            description: description.to_string(),
            items: None,
        },
    }
}

// ── Raw JSON wrapper ────────────────────────────────────────────────

/// A wrapper around a JSON string that serializes/deserializes as raw JSON.
/// Used for `tool_use` input fields where the Anthropic API expects a JSON
/// object, not a quoted string.
#[derive(Debug, Clone)]
pub struct RawJson(pub String);

impl RawJson {
    pub fn len(&self) -> usize {
        self.0.len()
    }
}

// Note: serde_json::Value is used transiently inside Serialize/Deserialize impls
// to bridge between the raw JSON string storage and the wire format. It never
// appears in any struct field or public API, respecting the project rule.

impl Serialize for RawJson {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let value: serde_json::Value =
            serde_json::from_str(&self.0).map_err(serde::ser::Error::custom)?;
        value.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for RawJson {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = serde_json::Value::deserialize(deserializer)?;
        Ok(Self(value.to_string()))
    }
}

// ── Chat content types ──────────────────────────────────────────────

/// Message content: either plain text or a sequence of content blocks.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ChatContent {
    Text(String),
    Blocks(Vec<ContentBlock>),
}

impl ChatContent {
    /// Extract the text content if this is a `Text` variant.
    pub fn text_content(&self) -> Option<&str> {
        match self {
            Self::Text(s) => Some(s),
            Self::Blocks(blocks) => blocks.iter().find_map(|b| match b {
                ContentBlock::Text { text } => Some(text.as_str()),
                _ => None,
            }),
        }
    }

    /// Approximate character length for token budget calculations.
    pub fn char_len(&self) -> usize {
        match self {
            Self::Text(s) => s.len(),
            Self::Blocks(blocks) => blocks
                .iter()
                .map(|b| match b {
                    ContentBlock::Text { text } => text.len(),
                    ContentBlock::ToolUse { ref input, .. } => input.len(),
                    ContentBlock::ToolResult { content, .. } => content.len(),
                })
                .sum(),
        }
    }
}

/// A content block within a message (for `tool_use` / `tool_result` payloads).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ContentBlock {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "tool_use")]
    ToolUse {
        id: String,
        name: String,
        input: RawJson,
    },
    #[serde(rename = "tool_result")]
    ToolResult {
        tool_use_id: String,
        content: String,
        is_error: bool,
    },
}
