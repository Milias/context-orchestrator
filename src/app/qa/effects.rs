//! Side-effects for the `ask` tool.
//!
//! Runs in `handle_tool_call_completed` after tool execution. Mutates the graph
//! only — creates `Question` nodes with `Asks` and `About` edges. Returns
//! enriched `ToolResultContent` with the question UUID. All routing and TUI
//! notifications happen via `GraphEvent` subscribers, not direct calls.

use crate::graph::node::{QuestionDestination, QuestionStatus};
use crate::graph::tool::result::ToolResultContent;
use crate::graph::tool::types::ToolCallArguments;
use crate::graph::{ConversationGraph, EdgeKind, Node};

use chrono::Utc;
use uuid::Uuid;

/// Apply Q/A side-effects for a completed tool call.
/// Returns enriched `ToolResultContent` if the tool is Q/A-related, `None` otherwise.
/// Only mutates the graph — no TUI state, no routing instructions.
pub fn apply(graph: &mut ConversationGraph, tool_call_id: Uuid) -> Option<ToolResultContent> {
    let arguments = match graph.node(tool_call_id)? {
        Node::ToolCall { arguments, .. } => arguments.clone(),
        _ => return None,
    };

    match &arguments {
        ToolCallArguments::Ask {
            question,
            destination,
            about_node_id,
            requires_approval,
        } => Some(apply_ask(
            graph,
            tool_call_id,
            question,
            *destination,
            *about_node_id,
            requires_approval.unwrap_or(false),
        )),
        _ => None,
    }
}

/// Create a `Question` node, wire `Asks` and `About` edges, return enriched content.
/// The `QuestionAdded` event is emitted by the `EventBus` when the node is added.
fn apply_ask(
    graph: &mut ConversationGraph,
    tool_call_id: Uuid,
    question: &str,
    destination: QuestionDestination,
    about_node_id: Option<Uuid>,
    requires_approval: bool,
) -> ToolResultContent {
    let question_id = Uuid::new_v4();
    let question_node = Node::Question {
        id: question_id,
        content: question.to_string(),
        destination,
        status: QuestionStatus::Pending,
        requires_approval,
        created_at: Utc::now(),
    };
    graph.add_node(question_node);
    graph.emit(crate::graph::event::GraphEvent::QuestionAdded {
        node_id: question_id,
        destination,
    });

    // Provenance: which tool call created this question.
    let _ = graph.add_edge(tool_call_id, question_id, EdgeKind::Asks);

    // Context: what the question is about (if the referenced node exists).
    if let Some(about_id) = about_node_id {
        if graph.node(about_id).is_some() {
            let _ = graph.add_edge(question_id, about_id, EdgeKind::About);
        }
    }

    ToolResultContent::text(format!(
        "Created question (id: {question_id}). Routing to {destination:?} backend."
    ))
}

#[cfg(test)]
#[path = "effects_tests.rs"]
mod tests;
