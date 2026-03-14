//! Context building and LLM-guided refinement for agent loops.
//!
//! Extracts messages from the graph via the agent's policy, optionally runs
//! a meta-LLM refinement pass (when `LlmGuided` mode is configured), then
//! counts tokens and sanitizes for the Anthropic API.
//!
//! Creates a `Node::ContextBuildingRequest` to record each context build
//! operation with provenance (`SelectedFor` edges to included nodes).

use crate::app::context;
use crate::app::SharedGraph;
use crate::graph::node::{ContextBuildStatus, ContextPolicyKind, ContextTrigger};
use crate::graph::{EdgeKind, Node};
use crate::llm::LlmProvider;
use crate::tasks::{AgentEvent, AgentPhase};

use super::streaming::AgentContext;
use super::AgentLoopConfig;

use chrono::Utc;
use std::sync::Arc;
use uuid::Uuid;

/// Result of context building: finalized messages plus the CBR node ID for provenance.
pub(in crate::app) struct ContextBuildOutput {
    /// System prompt for the LLM call.
    pub system_prompt: Option<String>,
    /// Ordered messages for the Anthropic API.
    pub messages: Vec<crate::llm::ChatMessage>,
    /// ID of the `ContextBuildingRequest` node created during this build.
    pub context_request_id: Uuid,
}

/// Build context from the graph via the agent's policy, then count tokens and sanitize.
/// When `LlmGuided` selection mode is configured, runs a meta-LLM refinement pass
/// on the scored candidates before finalizing.
/// Creates a `ContextBuildingRequest` node with `SelectedFor` edges for provenance.
/// Emits `BuildingContext` phase events for TUI feedback.
pub(in crate::app) async fn build_and_finalize_context(
    graph: &SharedGraph,
    provider: &Arc<dyn LlmProvider>,
    config: &AgentLoopConfig,
    ctx: &AgentContext,
) -> anyhow::Result<ContextBuildOutput> {
    let ctx_phase = Uuid::new_v4();
    ctx.send(AgentEvent::Progress {
        phase_id: ctx_phase,
        phase: AgentPhase::BuildingContext,
    });

    let (mut context_result, candidates_count) = {
        let g = graph.read();
        let raw_candidates = context::candidates::gather(
            &g,
            match &config.policy {
                context::policies::ContextPolicy::TaskExecution { work_item_id } => *work_item_id,
                context::policies::ContextPolicy::Conversational => {
                    g.active_leaf().unwrap_or_else(|_| Uuid::nil())
                }
            },
        );
        let count = raw_candidates.len();
        let result = context::extract_messages(&g, &config.policy, config.agent_id);
        (result, count)
    };

    // LLM-guided refinement: filter selected_node_ids to the LLM's choices.
    let refinement = run_llm_guided_refinement(graph, provider, config).await;
    if let Some(llm_ids) = refinement {
        context_result
            .selected_node_ids
            .retain(|id| llm_ids.contains(id));
    }

    tracing::debug!(
        agent_id = %config.agent_id,
        selected_nodes = context_result.selected_node_ids.len(),
        messages = context_result.messages.len(),
        "context pipeline selected nodes for agent"
    );

    let (system_prompt, messages) = context::finalize_context(
        context_result.system_prompt,
        context_result.messages,
        provider.as_ref(),
        &config.model,
        config.max_context_tokens,
        &config.tools,
    )
    .await?;

    // Create the ContextBuildingRequest node and SelectedFor edges.
    let cbr_id = {
        let trigger = match &config.policy {
            context::policies::ContextPolicy::Conversational => ContextTrigger::UserMessage,
            context::policies::ContextPolicy::TaskExecution { work_item_id } => {
                ContextTrigger::TaskExecution {
                    work_item_id: *work_item_id,
                }
            }
        };
        let policy_kind = match &config.policy {
            context::policies::ContextPolicy::Conversational => ContextPolicyKind::Conversational,
            context::policies::ContextPolicy::TaskExecution { .. } => {
                ContextPolicyKind::TaskExecution
            }
        };
        let cbr_id = Uuid::new_v4();
        let now = Utc::now();
        let cbr_node = Node::ContextBuildingRequest {
            id: cbr_id,
            trigger,
            policy: policy_kind,
            status: ContextBuildStatus::Built,
            candidates_count: u32::try_from(candidates_count).unwrap_or(u32::MAX),
            selected_count: u32::try_from(context_result.selected_node_ids.len())
                .unwrap_or(u32::MAX),
            token_count: None,
            agent_id: config.agent_id,
            created_at: now,
            built_at: Some(now),
        };

        let mut g = graph.write();
        g.add_node(cbr_node);
        for &selected_id in &context_result.selected_node_ids {
            let _ = g.add_edge(cbr_id, selected_id, EdgeKind::SelectedFor);
        }
        cbr_id
    };

    ctx.send(AgentEvent::PhaseCompleted {
        phase_id: ctx_phase,
    });

    Ok(ContextBuildOutput {
        system_prompt,
        messages,
        context_request_id: cbr_id,
    })
}

/// Run LLM-guided refinement on scored candidates when configured.
/// Returns `Some(selected_ids)` when the LLM successfully refined the selection,
/// `None` for heuristic mode, non-task-execution policies, or fallback.
async fn run_llm_guided_refinement(
    graph: &SharedGraph,
    provider: &Arc<dyn LlmProvider>,
    config: &AgentLoopConfig,
) -> Option<Vec<Uuid>> {
    if config.context_selection != crate::config::ContextSelectionMode::LlmGuided {
        return None;
    }
    let crate::app::context::policies::ContextPolicy::TaskExecution { work_item_id } =
        &config.policy
    else {
        return None;
    };
    let selector_model = config
        .context_selector_model
        .as_deref()
        .unwrap_or(&config.model);
    // Snapshot graph data under the lock, then release before the async selector call.
    let (scored, task_summary, graph_snapshot) = {
        let g = graph.read();
        let raw = context::candidates::gather(&g, *work_item_id);
        let scored = context::scoring::score_candidates(&g, *work_item_id, &raw);
        let summary = g
            .node(*work_item_id)
            .map_or("task", crate::graph::Node::content)
            .to_string();
        (scored, summary, g.clone())
    };
    let selection = context::selector::refine(
        provider,
        selector_model,
        &graph_snapshot,
        &scored,
        &task_summary,
    )
    .await;
    tracing::debug!(
        agent_id = %config.agent_id,
        selected = selection.selected_ids.len(),
        is_fallback = selection.is_fallback,
        "LLM-guided context selection completed"
    );

    // Only return the narrowed set when the LLM actually succeeded.
    if selection.is_fallback {
        None
    } else {
        Some(selection.selected_ids)
    }
}
