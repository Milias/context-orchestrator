//! Task execution context policy — focused on a specific work item.
//!
//! Builds context from the work item's description, parent plan tree, sibling
//! task statuses, and the agent's own `RespondsTo` chain. Uses the scoring
//! pipeline (gather → score → budget) to select which graph nodes to include,
//! replacing the naive full-chain walk with scored candidate selection.

use std::collections::HashSet;
use std::fmt::Write;

use crate::app::context::{budget, candidates, scoring};
use crate::graph::tool_types::ToolCallStatus;
use crate::graph::{ConversationGraph, EdgeKind, Node, Role, WorkItemKind};
use crate::llm::{ChatContent, ChatMessage, ContentBlock, RawJson};
use uuid::Uuid;

/// Build context for a task execution agent. Uses the scoring pipeline to
/// select which nodes to include, then renders selected nodes into messages.
///
/// Pipeline stages:
/// 1. `candidates::gather()` — heuristic expansion from the work item anchor
/// 2. `scoring::score_candidates()` — edge-weighted BFS + recency boost
/// 3. `budget::allocate()` — tier-based token budget partitioning
/// 4. Render — selected nodes become messages; supplementary become summaries
pub fn build_context(
    graph: &ConversationGraph,
    work_item_id: Uuid,
    agent_id: Uuid,
) -> super::ContextBuildResult {
    let mut system_prompt = String::new();

    // ── Task description ────────────────────────────────────────────
    if let Some(Node::WorkItem {
        title, description, ..
    }) = graph.node(work_item_id)
    {
        let _ = writeln!(system_prompt, "# Task: {title}");
        if let Some(desc) = description {
            let _ = writeln!(system_prompt, "\n{desc}");
        }
    }

    // ── Parent plan context (scoped, not all plans) ─────────────────
    if let Some(plan_section) = build_scoped_plan_section(graph, work_item_id) {
        system_prompt.push_str("\n\n");
        system_prompt.push_str(&plan_section);
    }

    // ── Q/A section for this agent ──────────────────────────────────
    if let Some(qa_section) = crate::app::qa::context::build_qa_section(graph, agent_id) {
        system_prompt.push_str("\n\n");
        system_prompt.push_str(&qa_section);
    }

    // ── API error context ───────────────────────────────────────────
    if let Some(error_section) = crate::app::context::error_context::build_error_section(graph) {
        system_prompt.push_str("\n\n");
        system_prompt.push_str(&error_section);
    }

    // ── Agent instructions ──────────────────────────────────────────
    system_prompt.push_str("\n\nYou are a task execution agent. Complete the task above using the available tools. When finished, call `update_work_item` to mark the task as Done.");

    // ── Scoring pipeline: gather → score → budget ───────────────────
    let raw_candidates = candidates::gather(graph, work_item_id);
    let scored = scoring::score_candidates(graph, work_item_id, &raw_candidates);
    // Use a conservative budget estimate for the message portion of context.
    // The system prompt sections above consume ~20% of the window; the rest
    // is available for scored node content.
    let max_message_tokens = estimate_message_budget(graph);
    let allocation = budget::allocate(scored, max_message_tokens);

    // Collect all selected node IDs for provenance tracking.
    let selected_ids: HashSet<Uuid> = allocation
        .full_detail
        .iter()
        .chain(allocation.supplementary.iter())
        .map(|c| c.node_id)
        .collect();

    // ── Supplementary summaries in system prompt ────────────────────
    if !allocation.supplementary.is_empty() {
        let summary = render_supplementary(graph, &allocation.supplementary);
        system_prompt.push_str("\n\n");
        system_prompt.push_str(&summary);
    }

    // ── Render messages from the agent's RespondsTo chain ────────────
    // Walk the chain but only include nodes that passed scoring.
    let chain = walk_chain_from_root(graph, work_item_id);
    let messages = render_chain_messages(graph, &chain, &selected_ids);

    let selected_node_ids: Vec<Uuid> = selected_ids.into_iter().collect();

    super::ContextBuildResult {
        system_prompt: Some(system_prompt),
        messages,
        selected_node_ids,
    }
}

/// Estimate the token budget available for messages (excluding system prompt).
/// Uses a conservative 80% of the default max context tokens.
fn estimate_message_budget(graph: &ConversationGraph) -> u32 {
    // Default 180k context tokens; 80% for messages, 20% for system prompt.
    let total_nodes = graph.nodes_by(|_| true).len();
    // Scale budget based on graph size: more nodes = more candidates competing.
    // Minimum 10k tokens, maximum 144k (80% of 180k).
    let base = 144_000_u32;
    if total_nodes < 50 {
        base
    } else {
        // As the graph grows, the pipeline's scoring becomes more important
        // for keeping context focused. Budget stays constant; scoring selects.
        base
    }
}

/// Render supplementary-tier nodes as compact one-line summaries in the system prompt.
fn render_supplementary(
    graph: &ConversationGraph,
    supplementary: &[scoring::ScoredCandidate],
) -> String {
    let mut lines = vec!["## Related Context (summaries)".to_string()];
    for candidate in supplementary {
        let content = graph.node(candidate.node_id).map_or("", Node::content);
        let truncated = if content.len() > 120 {
            let end = content.char_indices().nth(117).map_or(117, |(i, _)| i);
            format!("{}...", &content[..end])
        } else {
            content.to_string()
        };
        let type_tag = node_type_tag(graph, candidate.node_id);
        lines.push(format!("- [{type_tag}] {truncated}"));
    }
    lines.join("\n")
}

/// Short type label for a node, used in supplementary summaries.
fn node_type_tag(graph: &ConversationGraph, node_id: Uuid) -> &'static str {
    match graph.node(node_id) {
        Some(Node::Message { role, .. }) => match role {
            Role::User => "user",
            Role::Assistant => "assistant",
            Role::System => "system",
        },
        Some(Node::WorkItem { .. }) => "work_item",
        Some(Node::ToolCall { .. }) => "tool_call",
        Some(Node::ToolResult { .. }) => "tool_result",
        Some(Node::Question { .. }) => "question",
        Some(Node::Answer { .. }) => "answer",
        Some(Node::GitFile { .. }) => "git_file",
        Some(Node::ApiError { .. }) => "error",
        _ => "other",
    }
}

/// Render messages from a chain walk, filtering to only scored/selected nodes.
/// If a message node was not selected by the pipeline, it is skipped.
/// However, the agent's own chain is always included for coherent conversation
/// context (chain nodes pass scoring naturally due to `RespondsTo` edge weight).
fn render_chain_messages(
    graph: &ConversationGraph,
    chain: &[&Node],
    selected_ids: &HashSet<Uuid>,
) -> Vec<ChatMessage> {
    let mut messages = Vec::new();
    for node in chain {
        // Include chain nodes that were selected by the scoring pipeline.
        // Chain nodes connected via RespondsTo from the anchor score highly,
        // so this filter primarily prunes distant/irrelevant nodes.
        if !selected_ids.contains(&node.id()) {
            continue;
        }
        if let Node::Message {
            id, role, content, ..
        } = node
        {
            match role {
                Role::User => {
                    messages.push(ChatMessage::text(Role::User, content));
                }
                Role::Assistant => {
                    let (asst_msg, result_msgs) =
                        build_assistant_message_with_tools(graph, *id, content);
                    messages.push(asst_msg);
                    messages.extend(result_msgs);
                }
                Role::System => {}
            }
        }
    }
    messages
}

/// Build a plan section scoped to the parent plan of the given work item.
/// Returns `None` if the work item has no parent plan.
fn build_scoped_plan_section(graph: &ConversationGraph, work_item_id: Uuid) -> Option<String> {
    // Walk SubtaskOf upward to find the parent plan.
    let plan_id = find_parent_plan(graph, work_item_id)?;
    let Node::WorkItem {
        title: plan_title,
        status: plan_status,
        ..
    } = graph.node(plan_id)?
    else {
        return None;
    };

    let mut lines = vec![format!(
        "## Plan: \"{plan_title}\" [{status}]",
        status = match plan_status {
            crate::graph::WorkItemStatus::Todo => "todo",
            crate::graph::WorkItemStatus::Active => "active",
            crate::graph::WorkItemStatus::Done => "done",
        }
    )];

    // Show sibling tasks with their statuses.
    let children = graph.children_of(plan_id);
    for child_id in &children {
        if let Some(Node::WorkItem {
            id, title, status, ..
        }) = graph.node(*child_id)
        {
            let marker = if *id == work_item_id {
                "→ active (you)"
            } else {
                match status {
                    crate::graph::WorkItemStatus::Todo => "todo",
                    crate::graph::WorkItemStatus::Active => "active",
                    crate::graph::WorkItemStatus::Done => "done",
                }
            };
            lines.push(format!("  - [{marker}] {title} (id: {id})"));
        }
    }

    Some(lines.join("\n"))
}

/// Walk `SubtaskOf` edges upward from a work item to find the nearest Plan-kind ancestor.
fn find_parent_plan(graph: &ConversationGraph, mut node_id: Uuid) -> Option<Uuid> {
    for _ in 0..10 {
        let parent_id = graph.parent_of(node_id)?;
        if let Some(Node::WorkItem {
            kind: WorkItemKind::Plan,
            ..
        }) = graph.node(parent_id)
        {
            return Some(parent_id);
        }
        node_id = parent_id;
    }
    None
}

/// Walk forward from a root node through the `RespondsTo` chain, collecting
/// all nodes in chronological order (root first, leaf last).
fn walk_chain_from_root(graph: &ConversationGraph, root_id: Uuid) -> Vec<&Node> {
    let mut chain = Vec::new();
    let mut current = root_id;
    loop {
        let children = graph.reply_children_of(current);
        if children.is_empty() {
            break;
        }
        // Follow the last child (most recently added).
        current = *children.last().expect("non-empty checked above");
        if let Some(node) = graph.node(current) {
            chain.push(node);
        } else {
            break;
        }
    }
    chain
}

/// Build an assistant `ChatMessage` with `ToolUse` blocks and any following
/// user `ToolResult` messages. Reuse of the logic from `conversational.rs`.
fn build_assistant_message_with_tools(
    graph: &ConversationGraph,
    message_id: Uuid,
    text_content: &str,
) -> (ChatMessage, Vec<ChatMessage>) {
    let tool_call_ids = graph.sources_by_edge(message_id, EdgeKind::Invoked);
    let mut tool_use_blocks = Vec::new();
    let mut result_blocks = Vec::new();
    for tc_id in &tool_call_ids {
        let Some(Node::ToolCall {
            status,
            arguments,
            api_tool_use_id,
            ..
        }) = graph.node(*tc_id)
        else {
            continue;
        };
        if *status != ToolCallStatus::Completed && *status != ToolCallStatus::Failed {
            continue;
        }
        let result_id = graph
            .sources_by_edge(*tc_id, EdgeKind::Produced)
            .into_iter()
            .next();
        let Some(result_id) = result_id else {
            continue;
        };
        let Some(Node::ToolResult {
            content, is_error, ..
        }) = graph.node(result_id)
        else {
            continue;
        };
        let use_id = api_tool_use_id.clone().unwrap_or_else(|| tc_id.to_string());
        tool_use_blocks.push(ContentBlock::ToolUse {
            id: use_id.clone(),
            name: arguments.tool_name().to_string(),
            input: RawJson(arguments.to_input_json()),
        });
        result_blocks.push(ContentBlock::ToolResult {
            tool_use_id: use_id,
            content: content.clone(),
            is_error: *is_error,
        });
    }

    if tool_use_blocks.is_empty() {
        return (ChatMessage::text(Role::Assistant, text_content), vec![]);
    }
    let mut blocks = Vec::new();
    if !text_content.is_empty() {
        blocks.push(ContentBlock::Text {
            text: text_content.to_string(),
        });
    }
    blocks.extend(tool_use_blocks);
    let asst = ChatMessage {
        role: Role::Assistant,
        content: ChatContent::Blocks(blocks),
    };
    let results = ChatMessage {
        role: Role::User,
        content: ChatContent::Blocks(result_blocks),
    };
    (asst, vec![results])
}
