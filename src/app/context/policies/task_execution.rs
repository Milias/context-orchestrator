//! Task execution context policy — focused on a specific work item.
//!
//! Builds context from the work item's description, parent plan tree, sibling
//! task statuses, and the agent's own `RespondsTo` chain. Scopes plan context
//! to the parent plan only (not all plans in the graph).

use crate::graph::tool_types::ToolCallStatus;
use crate::graph::{ConversationGraph, EdgeKind, Node, Role, WorkItemKind};
use crate::llm::{ChatContent, ChatMessage, ContentBlock, RawJson};
use uuid::Uuid;

/// Build context for a task execution agent. Gathers the work item description,
/// parent plan context, sibling task statuses, and the agent's own conversation
/// chain rooted at the work item.
pub fn build_context(
    graph: &ConversationGraph,
    work_item_id: Uuid,
    agent_id: Uuid,
) -> super::ContextBuildResult {
    let mut system_prompt = String::new();
    let mut messages = Vec::new();
    let mut selected_node_ids = vec![work_item_id];

    // ── Task description ────────────────────────────────────────────
    if let Some(Node::WorkItem {
        title, description, ..
    }) = graph.node(work_item_id)
    {
        system_prompt.push_str(&format!("# Task: {title}\n"));
        if let Some(desc) = description {
            system_prompt.push_str(&format!("\n{desc}\n"));
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

    // ── Walk the agent's own RespondsTo chain from the work item ────
    let chain = walk_chain_from_root(graph, work_item_id);
    for node in &chain {
        selected_node_ids.push(node.id());
        match node {
            Node::Message {
                id, role, content, ..
            } => match role {
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
            },
            _ => {}
        }
    }

    super::ContextBuildResult {
        system_prompt: Some(system_prompt),
        messages,
        selected_node_ids,
    }
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
fn walk_chain_from_root<'a>(graph: &'a ConversationGraph, root_id: Uuid) -> Vec<&'a Node> {
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
