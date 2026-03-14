//! Node UUID resolution for graph navigation actions.
//!
//! Mirrors each section renderer's tree-flattening logic to produce
//! a flat list of node UUIDs in display order. Given the selected
//! index from [`ExplorerState`], we can resolve which graph node the
//! user is pointing at without coupling the input layer to rendering.

use uuid::Uuid;

use crate::graph::node::QuestionStatus;
use crate::graph::{ConversationGraph, EdgeKind, Node, Role};
use crate::tui::state::GraphSection;
use crate::tui::tabs::explorer::ExplorerState;

/// Resolve the UUID of the currently selected node in the active section.
///
/// Returns `None` if the section has no visible items or the selected
/// index is out of bounds.
pub fn resolve_selected_node_id(
    tui_state: &crate::tui::TuiState,
    section: GraphSection,
    graph: &ConversationGraph,
) -> Option<Uuid> {
    let explorer = tui_state.explorer.get(&section)?;
    let selected = explorer.selected;

    let ids = match section {
        GraphSection::Work => collect_work_ids(graph, explorer),
        GraphSection::QA => collect_qa_ids(graph, explorer),
        GraphSection::Execution => collect_execution_ids(graph, explorer),
        GraphSection::Context => collect_context_ids(graph, explorer),
    };

    ids.get(selected).copied()
}

// ── Work section ─────────────────────────────────────────────────────

/// Collect visible work-item UUIDs in the same order the renderer builds them.
fn collect_work_ids(graph: &ConversationGraph, explorer: &ExplorerState) -> Vec<Uuid> {
    let work_items: Vec<&Node> = graph.nodes_by(|n| matches!(n, Node::WorkItem { .. }));
    let mut roots: Vec<&Node> = work_items
        .iter()
        .filter(|n| graph.parent_of(n.id()).is_none())
        .copied()
        .collect();
    roots.sort_by(|a, b| {
        work_kind_order(a)
            .cmp(&work_kind_order(b))
            .then_with(|| a.content().cmp(b.content()))
    });

    let mut ids = Vec::new();
    for root in &roots {
        collect_work_subtree(graph, explorer, root, &mut ids);
    }
    ids
}

/// Sort key for work items: plans before tasks.
fn work_kind_order(node: &Node) -> u8 {
    match node {
        Node::WorkItem {
            kind: crate::graph::node::WorkItemKind::Plan,
            ..
        } => 0,
        _ => 1,
    }
}

/// Recursively collect node UUIDs in tree-walk order, respecting collapse state.
fn collect_work_subtree(
    graph: &ConversationGraph,
    explorer: &ExplorerState,
    node: &Node,
    out: &mut Vec<Uuid>,
) {
    let id = node.id();
    out.push(id);

    if explorer.is_collapsed(&id) {
        return;
    }

    let child_ids = graph.children_of(id);
    let mut children: Vec<&Node> = child_ids
        .iter()
        .filter_map(|cid| graph.node(*cid))
        .collect();
    children.sort_by(|a, b| {
        work_kind_order(a)
            .cmp(&work_kind_order(b))
            .then_with(|| a.content().cmp(b.content()))
    });

    for child in &children {
        collect_work_subtree(graph, explorer, child, out);
    }
}

// ── QA section ───────────────────────────────────────────────────────

/// Collect visible Q&A node UUIDs in display order.
fn collect_qa_ids(graph: &ConversationGraph, explorer: &ExplorerState) -> Vec<Uuid> {
    let mut questions: Vec<&Node> = graph.nodes_by(|n| matches!(n, Node::Question { .. }));
    questions.sort_by(|a, b| {
        qa_status_key(a)
            .cmp(&qa_status_key(b))
            .then_with(|| b.created_at().cmp(&a.created_at()))
    });

    let mut ids = Vec::new();
    for q in &questions {
        let qid = q.id();
        ids.push(qid);

        if explorer.is_collapsed(&qid) {
            continue;
        }

        let answer_ids = graph.sources_by_edge(qid, EdgeKind::Answers);
        let mut answers: Vec<&Node> = answer_ids
            .iter()
            .filter_map(|aid| graph.node(*aid))
            .collect();
        answers.sort_by_key(|a| a.created_at());

        for a in &answers {
            ids.push(a.id());
        }
    }
    ids
}

/// Sort key for question status: open first, then answered, then terminal.
fn qa_status_key(node: &Node) -> u8 {
    match node {
        Node::Question { status, .. } => match status {
            QuestionStatus::Pending | QuestionStatus::Claimed | QuestionStatus::PendingApproval => {
                0
            }
            QuestionStatus::Answered => 1,
            QuestionStatus::TimedOut | QuestionStatus::Rejected => 2,
        },
        _ => 3,
    }
}

// ── Execution section ────────────────────────────────────────────────

/// Collect visible execution node UUIDs in display order.
fn collect_execution_ids(graph: &ConversationGraph, explorer: &ExplorerState) -> Vec<Uuid> {
    use std::cmp::Reverse;

    let mut roots: Vec<&Node> = graph
        .nodes_by(|n| {
            matches!(
                n,
                Node::Message {
                    role: Role::Assistant,
                    ..
                }
            )
        })
        .into_iter()
        .collect();
    roots.sort_by_key(|n| Reverse(n.created_at()));

    let mut ids = Vec::new();
    for root in &roots {
        let root_id = root.id();
        ids.push(root_id);

        if explorer.is_collapsed(&root_id) {
            continue;
        }

        let tc_ids = graph.sources_by_edge(root_id, EdgeKind::Invoked);
        for tc_id in &tc_ids {
            ids.push(*tc_id);

            if explorer.is_collapsed(tc_id) {
                continue;
            }

            let result_ids = graph.sources_by_edge(*tc_id, EdgeKind::Produced);
            for tr_id in &result_ids {
                ids.push(*tr_id);
            }
        }
    }
    ids
}

// ── Context section ──────────────────────────────────────────────────

/// Collect visible context node UUIDs in display order.
///
/// Tier group headers (virtual items with no real graph node) are
/// represented by [`Uuid::nil()`] as a sentinel.
fn collect_context_ids(graph: &ConversationGraph, explorer: &ExplorerState) -> Vec<Uuid> {
    use std::cmp::Reverse;

    let mut cbr_nodes: Vec<&Node> =
        graph.nodes_by(|n| matches!(n, Node::ContextBuildingRequest { .. }));
    cbr_nodes.sort_by_key(|n| Reverse(n.created_at()));

    let mut ids = Vec::new();
    for cbr in &cbr_nodes {
        let cbr_id = cbr.id();
        ids.push(cbr_id);

        if explorer.is_collapsed(&cbr_id) {
            continue;
        }

        let selected_ids = graph.targets_by_edge(cbr_id, EdgeKind::SelectedFor);
        let (essential, important, supplementary) = classify_context_tiers(graph, &selected_ids);

        for tier_nodes in [&essential, &important, &supplementary] {
            if tier_nodes.is_empty() {
                continue;
            }
            // Tier header: sentinel nil UUID.
            ids.push(Uuid::nil());
            for node in tier_nodes {
                ids.push(node.id());
            }
        }
    }
    ids
}

/// Classify selected nodes into tier groups, matching the renderer's logic.
fn classify_context_tiers<'g>(
    graph: &'g ConversationGraph,
    selected_ids: &[Uuid],
) -> (Vec<&'g Node>, Vec<&'g Node>, Vec<&'g Node>) {
    let mut essential = Vec::new();
    let mut important = Vec::new();
    let mut supplementary = Vec::new();

    for &id in selected_ids {
        let Some(node) = graph.node(id) else {
            continue;
        };
        match node {
            Node::Message { .. } | Node::WorkItem { .. } => essential.push(node),
            Node::ToolCall { .. } | Node::ToolResult { .. } => important.push(node),
            _ => supplementary.push(node),
        }
    }

    (essential, important, supplementary)
}
