//! Q/A context injection — surfaces claimed questions in the system prompt.
//!
//! Similar to `plan::context::build_plan_section()`, this builds a system prompt
//! section listing questions claimed by the current agent. The agent sees them
//! and responds via the `answer` tool.

use crate::graph::node::QuestionStatus;
use crate::graph::{ConversationGraph, EdgeKind, Node};
use std::fmt::Write;
use uuid::Uuid;

/// Build a system prompt section listing questions claimed by this agent.
/// Returns `None` if no questions need answering.
pub fn build_qa_section(graph: &ConversationGraph, agent_id: Uuid) -> Option<String> {
    let questions: Vec<_> = graph
        .open_questions()
        .into_iter()
        .filter(|n| {
            matches!(
                n,
                Node::Question {
                    status: QuestionStatus::Claimed,
                    ..
                }
            )
        })
        .filter(|n| {
            graph
                .edges
                .iter()
                .any(|e| e.from == n.id() && e.kind == EdgeKind::ClaimedBy && e.to == agent_id)
        })
        .collect();

    if questions.is_empty() {
        return None;
    }

    let mut section = String::from("## Pending Questions (awaiting your answer)\n\n");
    section.push_str("Use the `answer` tool to respond to each question.\n\n");
    for q in &questions {
        if let Node::Question { id, content, .. } = q {
            let _ = writeln!(section, "- Question (id: {id}): {content}");
        }
    }
    Some(section)
}

#[cfg(test)]
#[path = "context_tests.rs"]
mod tests;
