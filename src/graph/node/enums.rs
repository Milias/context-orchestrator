//! Supporting enums for graph nodes and edges.

use serde::{Deserialize, Serialize};

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

/// Whether a work item is a top-level plan or a task within a plan.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum WorkItemKind {
    Plan,
    #[default]
    Task,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
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

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Pending,
    Running,
    Completed,
    Failed,
    Stopped,
}

/// Routing destination for a question.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum QuestionDestination {
    User,
    Llm,
    Auto,
}

/// Lifecycle state machine for `Question` nodes.
///
/// ```text
/// Pending ──try_claim()──→ Claimed ──add_answer()──→ Answered (if !requires_approval)
///    │                        │
///    ├──timeout──→ TimedOut   └──add_answer()──→ PendingApproval
///                                                    │
///                                  accept──→ Answered │
///                                  reject──→ Rejected ──→ Pending (re-claimable)
/// ```
///
/// Transitions are validated by `update_question_status()` in `mutation.rs`.
/// Invalid transitions (e.g., `Pending` → `Answered`) return `Err`.
/// `Rejected` is transient — it immediately transitions back to `Pending`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum QuestionStatus {
    /// Awaiting routing/claiming by a backend.
    Pending,
    /// Claimed by an agent or backend; answer in progress.
    Claimed,
    /// Answer produced but requires user approval before resolving.
    PendingApproval,
    /// Terminal: question fully resolved.
    Answered,
    /// Transient: user rejected the proposed answer. Returns to `Pending`.
    Rejected,
    /// Terminal: question expired without an answer.
    TimedOut,
}

// ── Context Building ────────────────────────────────────────────────

/// Lifecycle status of a context building operation.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ContextBuildStatus {
    /// Request created, traversal not yet started.
    Requested,
    /// Graph traversal and node selection in progress.
    Building,
    /// Context fully constructed and rendered.
    Built,
    /// Context was consumed by an LLM call.
    Consumed,
    /// Context built via heuristic fallback (LLM selection failed).
    FallbackUsed,
    /// Context building failed (e.g., empty graph, policy error).
    Failed,
}

/// What triggered a context building operation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ContextTrigger {
    /// User sent a message on the conversation branch.
    UserMessage,
    /// Agent is executing a work item.
    TaskExecution { work_item_id: uuid::Uuid },
    /// Agent is answering a question.
    QuestionResponse { question_id: uuid::Uuid },
}

/// Which context policy was used for a context building operation.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ContextPolicyKind {
    Conversational,
    TaskExecution,
}

// ── Edge + EdgeKind ─────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EdgeKind {
    RespondsTo,
    SubtaskOf,
    RelevantTo,
    /// Plan-to-plan dependency: `from` depends on `to` completing first.
    DependsOn,
    Tracks,
    Indexes,
    Provides,
    ThinkingOf,
    Invoked,
    Produced,
    /// `ToolCall` → `Question`: provenance of who asked.
    Asks,
    /// `Answer` → `Question`: resolution link.
    Answers,
    /// `Question` → any node: what the question references.
    About,
    /// `Answer` → any node: what the answer caused.
    Triggers,
    /// `Answer` → `Answer`: newer answer replaces older for the same question.
    Supersedes,
    /// Any node → agent UUID: coordination lock preventing double-execution.
    ClaimedBy,
    /// `ApiError` → branch leaf: records when the error occurred in the conversation.
    OccurredDuring,
    /// `ContextBuildingRequest` → any node: this node was included in the context window.
    SelectedFor,
    /// `ContextBuildingRequest` → `Message` (assistant): this context produced this response.
    ConsumedBy,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Edge {
    pub from: uuid::Uuid,
    pub to: uuid::Uuid,
    pub kind: EdgeKind,
}
