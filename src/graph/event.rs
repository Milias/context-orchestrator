//! Event bus: semantic events that drive all cross-component communication.
//!
//! Every variant represents a meaningful domain operation. Subscribers (TUI,
//! router, scheduler) react to events independently. Graph-mutation events are
//! emitted by mutation methods; agent/analytics events are emitted by the main
//! loop after processing `TaskMessage`s.
//!
//! The bus lives inside `ConversationGraph` as an `Option<EventBus>` field,
//! following the `#[serde(skip)]` pattern of `responds_to` and `invoked_by`.

use crate::graph::node::{QuestionDestination, QuestionStatus, WorkItemKind};
use crate::graph::{Role, StopReason, TaskStatus, WorkItemStatus};
use crate::tasks::AgentPhase;

use tokio::sync::broadcast;
use uuid::Uuid;

/// Buffer size for the broadcast channel. Accommodates burst scenarios
/// (e.g., git watcher replacing 200+ file nodes) while keeping memory bounded.
const EVENT_BUFFER_SIZE: usize = 256;

/// Semantic events emitted by graph mutations.
/// All fields are `Copy` types — clone cost is negligible.
#[derive(Debug, Clone)]
pub enum GraphEvent {
    /// A message (user or assistant) was added to the conversation.
    MessageAdded { node_id: Uuid, role: Role },
    /// A tool call completed (result node added, status updated).
    ToolCallCompleted { node_id: Uuid, is_error: bool },
    /// A work item (plan or task) was created.
    WorkItemAdded { node_id: Uuid, kind: WorkItemKind },
    /// A work item's status changed (includes upward propagation).
    WorkItemStatusChanged {
        node_id: Uuid,
        new_status: WorkItemStatus,
    },
    /// A question was created and needs routing.
    QuestionAdded {
        node_id: Uuid,
        destination: QuestionDestination,
    },
    /// A question's lifecycle status changed (claimed, answered, rejected, etc.).
    /// Emitted for every valid transition in the state machine.
    QuestionStatusChanged {
        node_id: Uuid,
        new_status: QuestionStatus,
    },
    /// A question was fully answered (terminal resolution, unblocks dependencies).
    /// Emitted in addition to `QuestionStatusChanged` — carries the `answer_id`
    /// that `QuestionStatusChanged` does not.
    QuestionAnswered { question_id: Uuid, answer_id: Uuid },
    /// A node was claimed by an agent.
    NodeClaimed { node_id: Uuid, agent_id: Uuid },
    /// Git file nodes were bulk-replaced.
    GitFilesRefreshed { count: usize },
    /// Tool nodes were bulk-replaced.
    ToolsRefreshed { count: usize },
    /// A background task's status changed.
    BackgroundTaskChanged { node_id: Uuid, status: TaskStatus },
    /// A dependency edge was added between work items.
    DependencyAdded { from_id: Uuid, to_id: Uuid },
    /// A task agent proposed completion with a confidence level.
    CompletionProposed {
        node_id: Uuid,
        confidence: crate::graph::node::CompletionConfidence,
    },

    // ── Agent lifecycle events ──────────────────────────────────────
    /// Agent loop phase changed (preparing, streaming, executing tools).
    AgentPhaseChanged { agent_id: Uuid, phase: AgentPhase },
    /// New accumulated streaming text from the LLM.
    StreamDelta {
        agent_id: Uuid,
        text: String,
        is_thinking: bool,
    },
    /// Agent committed an assistant message to the graph.
    AgentIterationCommitted {
        agent_id: Uuid,
        assistant_id: Uuid,
        stop_reason: Option<StopReason>,
    },
    /// Agent finished execution and terminated (ephemeral model).
    AgentFinished { agent_id: Uuid },

    // ── System events ───────────────────────────────────────────────
    /// A question was routed to the user for answering. TUI should show the prompt.
    QuestionRoutedToUser { question_id: Uuid, content: String },
    /// A non-fatal error to display in the status bar.
    ErrorOccurred { message: String },
    /// Fresh lifetime token totals from the analytics DB.
    TokenTotalsUpdated { input: u64, output: u64 },
}

/// Broadcast sender for graph events. Runtime-only (not serialized).
/// Wraps `tokio::broadcast::Sender` with a fixed buffer size.
#[derive(Debug, Clone)]
pub struct EventBus {
    tx: broadcast::Sender<GraphEvent>,
}

impl EventBus {
    /// Create a new event bus with the default buffer size.
    pub fn new() -> Self {
        let (tx, _) = broadcast::channel(EVENT_BUFFER_SIZE);
        Self { tx }
    }

    /// Subscribe to graph events. Returns a receiver that will get all
    /// future events. Lagged receivers get `RecvError::Lagged` and can
    /// recover by re-reading the graph snapshot.
    pub fn subscribe(&self) -> broadcast::Receiver<GraphEvent> {
        self.tx.subscribe()
    }

    /// Emit an event to all subscribers. Silently ignores errors
    /// (no subscribers = no-op, which is fine for tests and persistence).
    pub fn emit(&self, event: GraphEvent) {
        let _ = self.tx.send(event);
    }
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new()
    }
}
