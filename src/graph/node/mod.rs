//! Graph node types, supporting enums, and edge types.

mod enums;

pub use enums::*;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::tool_types::{ToolCallArguments, ToolCallStatus, ToolResultContent};

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
        /// Plan (top-level container) or Task (actionable item within a plan).
        #[serde(default)]
        kind: WorkItemKind,
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
    /// A question surfaced by an agent for routing to a backend (user, LLM, auto).
    Question {
        id: Uuid,
        content: String,
        destination: QuestionDestination,
        status: QuestionStatus,
        requires_approval: bool,
        created_at: DateTime<Utc>,
    },
    /// An answer resolving a `Question`. Linked via `Answers` edge.
    /// `question_id` is denormalized convenience (canonical link is the edge).
    Answer {
        id: Uuid,
        content: String,
        question_id: Uuid,
        created_at: DateTime<Utc>,
    },
    /// A non-retryable API error recorded in the graph for system prompt injection.
    /// Linked to the branch leaf at time of failure via `OccurredDuring` edge.
    /// Not part of the conversation branch (`RespondsTo`) — surfaced via system prompt.
    ApiError {
        id: Uuid,
        message: String,
        created_at: DateTime<Utc>,
    },
    /// Records a context construction operation. Makes context building
    /// observable and enables LLM-guided selection via `SelectedFor` edges.
    ContextBuildingRequest {
        id: Uuid,
        /// What triggered this context build.
        trigger: ContextTrigger,
        /// Which context policy was used.
        policy: ContextPolicyKind,
        /// Current lifecycle status.
        status: ContextBuildStatus,
        /// Number of candidate nodes considered.
        candidates_count: u32,
        /// Number of nodes selected for the context window.
        selected_count: u32,
        /// Total tokens in the rendered context.
        token_count: Option<u32>,
        /// Agent that will consume this context.
        agent_id: Uuid,
        created_at: DateTime<Utc>,
        /// When the context was fully built.
        #[serde(default)]
        built_at: Option<DateTime<Utc>>,
    },
}

impl Node {
    /// Unique identifier for this node.
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
            | Node::ToolResult { id, .. }
            | Node::Question { id, .. }
            | Node::Answer { id, .. }
            | Node::ApiError { id, .. }
            | Node::ContextBuildingRequest { id, .. } => *id,
        }
    }

    /// Primary content of this node for display purposes.
    pub fn content(&self) -> &str {
        match self {
            Node::Message { content, .. }
            | Node::SystemDirective { content, .. }
            | Node::ThinkBlock { content, .. }
            | Node::Question { content, .. }
            | Node::Answer { content, .. } => content,
            Node::ApiError { message, .. } => message,
            Node::ToolResult { content, .. } => content.text_content(),
            Node::WorkItem { title, .. } => title,
            Node::GitFile { path, .. } => path,
            Node::Tool { name, .. } => name,
            Node::BackgroundTask { description, .. } => description,
            Node::ToolCall { arguments, .. } => arguments.tool_name(),
            Node::ContextBuildingRequest { policy, .. } => match policy {
                ContextPolicyKind::Conversational => "context:conversational",
                ContextPolicyKind::TaskExecution => "context:task_execution",
            },
        }
    }

    /// Token count from provider token counting (input tokens for messages).
    pub fn input_tokens(&self) -> Option<u32> {
        match self {
            Node::Message { input_tokens, .. } => *input_tokens,
            _ => None,
        }
    }

    /// Token count from LLM output.
    pub fn output_tokens(&self) -> Option<u32> {
        match self {
            Node::Message { output_tokens, .. } => *output_tokens,
            _ => None,
        }
    }

    /// When this node was created.
    pub fn created_at(&self) -> DateTime<Utc> {
        match self {
            Node::Message { created_at, .. }
            | Node::SystemDirective { created_at, .. }
            | Node::ThinkBlock { created_at, .. }
            | Node::ToolCall { created_at, .. }
            | Node::ToolResult { created_at, .. }
            | Node::WorkItem { created_at, .. }
            | Node::BackgroundTask { created_at, .. }
            | Node::Question { created_at, .. }
            | Node::Answer { created_at, .. }
            | Node::ApiError { created_at, .. }
            | Node::ContextBuildingRequest { created_at, .. } => *created_at,
            Node::GitFile { updated_at, .. } | Node::Tool { updated_at, .. } => *updated_at,
        }
    }

    /// Model name for assistant messages, `None` otherwise.
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

#[cfg(test)]
#[path = "../node_tests.rs"]
mod node_tests;
