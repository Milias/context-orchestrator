use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::tool_types::{ToolCallArguments, ToolCallStatus, ToolResultContent};

// ── Enums ────────────────────────────────────────────────────────────

/// The reason the LLM stopped generating output.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    EndTurn,
    MaxTokens,
    ToolUse,
}

impl StopReason {
    /// Convert an API stop reason string (e.g. `"end_turn"`) into a typed enum.
    /// Returns `None` for unknown/future values.
    pub fn from_api(s: &str) -> Option<Self> {
        match s {
            "end_turn" => Some(Self::EndTurn),
            "max_tokens" => Some(Self::MaxTokens),
            "tool_use" => Some(Self::ToolUse),
            _ => None,
        }
    }
}

/// The role of a message sender in a conversation.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    User,
    Assistant,
    System,
}

impl std::fmt::Display for Role {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::User => write!(f, "user"),
            Self::Assistant => write!(f, "assistant"),
            Self::System => write!(f, "system"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WorkItemStatus {
    Todo,
    Active,
    Done,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GitFileStatus {
    Tracked,
    Modified,
    Staged,
    Untracked,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BackgroundTaskKind {
    GitIndex,
    ContextSummarize,
    ToolDiscovery,
    AgentPhase,
}

impl BackgroundTaskKind {
    pub fn is_service(&self) -> bool {
        matches!(
            self,
            Self::GitIndex | Self::ToolDiscovery | Self::ContextSummarize
        )
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Pending,
    Running,
    Completed,
    Failed,
    Stopped,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EdgeKind {
    RespondsTo,
    SubtaskOf,
    RelevantTo,
    Tracks,
    Indexes,
    Provides,
    ThinkingOf,
    Invoked,
    Produced,
}

// ── Edge ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Edge {
    pub from: Uuid,
    pub to: Uuid,
    pub kind: EdgeKind,
}

// ── Node ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Node {
    Message {
        id: Uuid,
        role: Role,
        content: String,
        created_at: DateTime<Utc>,
        model: Option<String>,
        input_tokens: Option<u32>,
        output_tokens: Option<u32>,
        /// Why the LLM stopped generating. `None` for user/system messages
        /// and for messages loaded from older graph formats.
        #[serde(default)]
        stop_reason: Option<StopReason>,
    },
    SystemDirective {
        id: Uuid,
        content: String,
        created_at: DateTime<Utc>,
    },
    WorkItem {
        id: Uuid,
        title: String,
        status: WorkItemStatus,
        description: Option<String>,
        created_at: DateTime<Utc>,
    },
    GitFile {
        id: Uuid,
        path: String,
        status: GitFileStatus,
        updated_at: DateTime<Utc>,
    },
    Tool {
        id: Uuid,
        name: String,
        description: String,
        updated_at: DateTime<Utc>,
    },
    BackgroundTask {
        id: Uuid,
        kind: BackgroundTaskKind,
        status: TaskStatus,
        description: String,
        created_at: DateTime<Utc>,
        updated_at: DateTime<Utc>,
    },
    ThinkBlock {
        id: Uuid,
        content: String,
        parent_message_id: Uuid,
        created_at: DateTime<Utc>,
    },
    ToolCall {
        id: Uuid,
        /// The API-assigned `tool_use` ID (e.g. `toolu_xxx`), used to pair
        /// `tool_use`/`tool_result` blocks in the LLM context.
        api_tool_use_id: Option<String>,
        arguments: ToolCallArguments,
        status: ToolCallStatus,
        parent_message_id: Uuid,
        created_at: DateTime<Utc>,
        completed_at: Option<DateTime<Utc>>,
    },
    ToolResult {
        id: Uuid,
        tool_call_id: Uuid,
        content: ToolResultContent,
        is_error: bool,
        created_at: DateTime<Utc>,
    },
}

impl Node {
    pub fn id(&self) -> Uuid {
        match self {
            Node::Message { id, .. }
            | Node::SystemDirective { id, .. }
            | Node::WorkItem { id, .. }
            | Node::GitFile { id, .. }
            | Node::Tool { id, .. }
            | Node::BackgroundTask { id, .. }
            | Node::ThinkBlock { id, .. }
            | Node::ToolCall { id, .. }
            | Node::ToolResult { id, .. } => *id,
        }
    }

    pub fn content(&self) -> &str {
        match self {
            Node::Message { content, .. }
            | Node::SystemDirective { content, .. }
            | Node::ThinkBlock { content, .. } => content,
            Node::ToolResult { content, .. } => content.text_content(),
            Node::WorkItem { title, .. } => title,
            Node::GitFile { path, .. } => path,
            Node::Tool { name, .. } => name,
            Node::BackgroundTask { description, .. } => description,
            Node::ToolCall { arguments, .. } => arguments.tool_name(),
        }
    }

    pub fn input_tokens(&self) -> Option<u32> {
        match self {
            Node::Message { input_tokens, .. } => *input_tokens,
            _ => None,
        }
    }

    pub fn output_tokens(&self) -> Option<u32> {
        match self {
            Node::Message { output_tokens, .. } => *output_tokens,
            _ => None,
        }
    }

    pub fn created_at(&self) -> DateTime<Utc> {
        match self {
            Node::Message { created_at, .. }
            | Node::SystemDirective { created_at, .. }
            | Node::ThinkBlock { created_at, .. }
            | Node::ToolCall { created_at, .. }
            | Node::ToolResult { created_at, .. }
            | Node::WorkItem { created_at, .. }
            | Node::BackgroundTask { created_at, .. } => *created_at,
            Node::GitFile { updated_at, .. } | Node::Tool { updated_at, .. } => *updated_at,
        }
    }

    pub fn model(&self) -> Option<&str> {
        match self {
            Node::Message { model, .. } => model.as_deref(),
            _ => None,
        }
    }

    /// Returns the stop reason for assistant messages, `None` otherwise.
    pub fn stop_reason(&self) -> Option<StopReason> {
        match self {
            Node::Message { stop_reason, .. } => *stop_reason,
            _ => None,
        }
    }

    /// Whether this message was truncated due to `max_tokens`.
    pub fn is_truncated(&self) -> bool {
        self.stop_reason() == Some(StopReason::MaxTokens)
    }
}
