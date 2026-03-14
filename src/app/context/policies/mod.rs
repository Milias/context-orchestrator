//! Context policies for different agent roles.
//!
//! A [`ContextPolicy`] determines how an agent builds context from the graph,
//! how it records messages, and which tools it can use. All agents are
//! ephemeral — the policy is the only difference between them.

pub mod conversational;
pub mod task_execution;

use crate::graph::ConversationGraph;
use crate::llm::ChatMessage;
use uuid::Uuid;

/// Result of context extraction including provenance metadata.
/// Both the rendered context and the IDs of nodes that contributed to it.
pub struct ContextBuildResult {
    /// System prompt for the LLM (may include plan/QA/error sections).
    pub system_prompt: Option<String>,
    /// Ordered message list for the Anthropic API.
    pub messages: Vec<ChatMessage>,
    /// IDs of all graph nodes that contributed to this context window.
    /// Used to create `SelectedFor` edges on the `ContextBuildingRequest`.
    pub selected_node_ids: Vec<Uuid>,
}

/// Determines how context is built for an agent and how it interacts with the graph.
/// Each variant represents a different agent role with different traversal strategies.
pub enum ContextPolicy {
    /// Interactive chat on the main branch. Context = branch history walk.
    Conversational,
    /// Focused execution of a specific work item. Context = scoped subgraph.
    TaskExecution { work_item_id: Uuid },
}

impl ContextPolicy {
    /// Build context from the graph using this policy's traversal strategy.
    pub fn build_context(
        &self,
        graph: &ConversationGraph,
        agent_id: Uuid,
    ) -> ContextBuildResult {
        match self {
            Self::Conversational => conversational::build_context(graph, agent_id),
            Self::TaskExecution { work_item_id } => {
                task_execution::build_context(graph, *work_item_id, agent_id)
            }
        }
    }

    /// Get the initial parent node for this agent's first message.
    /// Conversational: active branch leaf. Task: the work item node.
    pub fn initial_parent(&self, graph: &ConversationGraph) -> anyhow::Result<Uuid> {
        match self {
            Self::Conversational => graph.active_leaf(),
            Self::TaskExecution { work_item_id } => {
                // The agent's chain starts from the work item's chain leaf
                // (which may be a synthetic user message if this is the first activation).
                Ok(graph.find_chain_leaf(*work_item_id))
            }
        }
    }

    /// Record an assistant message in the graph using the appropriate method.
    /// Conversational: `add_message` (updates branch). Task: `add_reply` (no branch update).
    pub fn record_message(
        &self,
        graph: &mut ConversationGraph,
        parent_id: Uuid,
        node: crate::graph::Node,
    ) -> anyhow::Result<Uuid> {
        match self {
            Self::Conversational => graph.add_message(parent_id, node),
            Self::TaskExecution { .. } => graph.add_reply(parent_id, node),
        }
    }

    /// Tool filter for this policy. Returns `None` for all tools (conversational)
    /// or `Some` with a whitelist of allowed tool names (task execution).
    pub fn tool_filter(&self) -> Option<&[&str]> {
        match self {
            Self::Conversational => None,
            Self::TaskExecution { .. } => Some(&[
                "read_file",
                "write_file",
                "list_directory",
                "search_files",
                "update_work_item",
                "ask",
                "answer",
            ]),
        }
    }
}
