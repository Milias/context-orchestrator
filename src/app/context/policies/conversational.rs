//! Conversational context policy — interactive chat with the user.
//!
//! Anchors on the active branch leaf, walks `RespondsTo` ancestors, includes
//! all messages verbatim. Produces identical output to the original
//! `extract_messages()` function (behavioral equivalence).

use crate::graph::{ConversationGraph, Node, Role};
use crate::llm::ChatMessage;
use uuid::Uuid;

/// Build context for a conversational agent. Walks the active branch history,
/// collects all contributing node IDs for provenance tracking.
pub fn build_context(graph: &ConversationGraph, agent_id: Uuid) -> super::ContextBuildResult {
    let history = graph
        .get_branch_history(graph.active_branch())
        .unwrap_or_default();

    let mut system_prompt = None;
    let mut messages = Vec::new();
    let mut selected_node_ids = Vec::new();

    for node in history {
        match node {
            Node::SystemDirective { id, content, .. } => {
                system_prompt = Some(content.clone());
                selected_node_ids.push(*id);
            }
            Node::Message {
                id, role, content, ..
            } => match role {
                Role::System => {}
                Role::User => {
                    messages.push(ChatMessage::text(Role::User, content));
                    selected_node_ids.push(*id);
                }
                Role::Assistant => {
                    let (asst_msg, result_msgs) =
                        super::message_builder::build_assistant_message_with_tools(
                            graph, *id, content,
                        );
                    messages.push(asst_msg);
                    messages.extend(result_msgs);
                    selected_node_ids.push(*id);
                }
            },
            Node::WorkItem { .. }
            | Node::GitFile { .. }
            | Node::Tool { .. }
            | Node::BackgroundTask { .. }
            | Node::ThinkBlock { .. }
            | Node::ToolCall { .. }
            | Node::ToolResult { .. }
            | Node::Question { .. }
            | Node::Answer { .. }
            | Node::ApiError { .. }
            | Node::ContextBuildingRequest { .. } => {}
        }
    }

    // Inject active plan context into the system prompt.
    if let Some(plan_section) = crate::app::plan::context::build_plan_section(graph) {
        let prompt = system_prompt.get_or_insert_with(String::new);
        prompt.push_str("\n\n");
        prompt.push_str(&plan_section);
    }

    // Inject pending Q/A context for the agent.
    if let Some(qa_section) = crate::app::qa::context::build_qa_section(graph, agent_id) {
        let prompt = system_prompt.get_or_insert_with(String::new);
        prompt.push_str("\n\n");
        prompt.push_str(&qa_section);
    }

    // Inject API error context so the LLM can adapt on retry.
    if let Some(error_section) = crate::app::context::error_context::build_error_section(graph) {
        let prompt = system_prompt.get_or_insert_with(String::new);
        prompt.push_str("\n\n");
        prompt.push_str(&error_section);
    }

    super::ContextBuildResult {
        system_prompt,
        messages,
        selected_node_ids,
    }
}

#[cfg(test)]
#[path = "conversational_tests.rs"]
mod tests;
