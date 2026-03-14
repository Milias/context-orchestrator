//! Context building and LLM-guided refinement for agent loops.
//!
//! Extracts messages from the graph via the agent's policy, optionally runs
//! a meta-LLM refinement pass (when `LlmGuided` mode is configured), then
//! counts tokens and sanitizes for the Anthropic API.

use crate::app::context;
use crate::app::SharedGraph;
use crate::llm::LlmProvider;
use crate::tasks::{AgentEvent, AgentPhase};

use super::streaming::AgentContext;
use super::AgentLoopConfig;

use std::sync::Arc;
use uuid::Uuid;

/// Build context from the graph via the agent's policy, then count tokens and sanitize.
/// When `LlmGuided` selection mode is configured, runs a meta-LLM refinement pass
/// on the scored candidates before finalizing.
/// Emits `BuildingContext` phase events for TUI feedback.
pub(in crate::app) async fn build_and_finalize_context(
    graph: &SharedGraph,
    provider: &Arc<dyn LlmProvider>,
    config: &AgentLoopConfig,
    ctx: &AgentContext,
) -> anyhow::Result<(Option<String>, Vec<crate::llm::ChatMessage>)> {
    let ctx_phase = Uuid::new_v4();
    ctx.send(AgentEvent::Progress {
        phase_id: ctx_phase,
        phase: AgentPhase::BuildingContext,
    });

    let context_result = {
        let g = graph.read();
        context::extract_messages(&g, &config.policy, config.agent_id)
    };

    run_llm_guided_refinement(graph, provider, config).await;

    tracing::debug!(
        agent_id = %config.agent_id,
        selected_nodes = context_result.selected_node_ids.len(),
        messages = context_result.messages.len(),
        "context pipeline selected nodes for agent"
    );
    let result = context::finalize_context(
        context_result.system_prompt,
        context_result.messages,
        provider.as_ref(),
        &config.model,
        config.max_context_tokens,
        &config.tools,
    )
    .await?;

    ctx.send(AgentEvent::PhaseCompleted {
        phase_id: ctx_phase,
    });

    Ok(result)
}

/// Run LLM-guided refinement on scored candidates when configured.
/// No-op for heuristic mode or non-task-execution policies.
async fn run_llm_guided_refinement(
    graph: &SharedGraph,
    provider: &Arc<dyn LlmProvider>,
    config: &AgentLoopConfig,
) {
    if config.context_selection != crate::config::ContextSelectionMode::LlmGuided {
        return;
    }
    let crate::app::context::policies::ContextPolicy::TaskExecution { work_item_id } =
        &config.policy
    else {
        return;
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
}
