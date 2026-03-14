use super::*;
use crate::graph::{ConversationGraph, EdgeKind, Node, WorkItemKind, WorkItemStatus};

use chrono::Utc;
use uuid::Uuid;

/// Bug: collapsed items incorrectly showing children in the visible list.
/// `flatten_visible` with an empty `expanded` set must return only the
/// root plan IDs — no children. If the expand/collapse gate is missing
/// or inverted, children leak into the visible list.
#[test]
fn test_flatten_visible_collapsed_hides_children() {
    let mut graph = ConversationGraph::new("sys");

    // Root plan.
    let plan_id = Uuid::new_v4();
    graph.add_node(Node::WorkItem {
        id: plan_id,
        kind: WorkItemKind::Plan,
        title: "Root plan".to_string(),
        status: WorkItemStatus::Active,
        description: None,
        created_at: Utc::now(),
    });

    // Two child tasks.
    let task1_id = Uuid::new_v4();
    let task2_id = Uuid::new_v4();
    for (id, title) in [(task1_id, "Task 1"), (task2_id, "Task 2")] {
        graph.add_node(Node::WorkItem {
            id,
            kind: WorkItemKind::Task,
            title: title.to_string(),
            status: WorkItemStatus::Todo,
            description: None,
            created_at: Utc::now(),
        });
        graph.add_edge(id, plan_id, EdgeKind::SubtaskOf).unwrap();
    }

    // Default state: nothing expanded.
    let state = WorkTreeState::default();
    let visible = flatten_visible(&graph, &state);

    assert_eq!(
        visible,
        vec![plan_id],
        "collapsed plan should show only the plan itself, not its children"
    );
}

/// Bug: expanded items missing children in the visible list.
/// When a plan is in the `expanded` set, `flatten_visible` must include
/// both the plan and its children. If the children query or the expansion
/// guard is broken, children will be absent despite the plan being expanded.
#[test]
fn test_flatten_visible_expanded_shows_children() {
    let mut graph = ConversationGraph::new("sys");

    // Root plan.
    let plan_id = Uuid::new_v4();
    graph.add_node(Node::WorkItem {
        id: plan_id,
        kind: WorkItemKind::Plan,
        title: "Root plan".to_string(),
        status: WorkItemStatus::Active,
        description: None,
        created_at: Utc::now(),
    });

    // Two child tasks.
    let task1_id = Uuid::new_v4();
    let task2_id = Uuid::new_v4();
    for (id, title) in [(task1_id, "Task 1"), (task2_id, "Task 2")] {
        graph.add_node(Node::WorkItem {
            id,
            kind: WorkItemKind::Task,
            title: title.to_string(),
            status: WorkItemStatus::Todo,
            description: None,
            created_at: Utc::now(),
        });
        graph.add_edge(id, plan_id, EdgeKind::SubtaskOf).unwrap();
    }

    // Expand the plan.
    let mut state = WorkTreeState::default();
    state.expanded.insert(plan_id);
    let visible = flatten_visible(&graph, &state);

    // Plan must be first, followed by both children.
    assert_eq!(visible[0], plan_id, "plan should be the first visible item");
    assert_eq!(
        visible.len(),
        3,
        "expanded plan with 2 children should produce 3 visible items"
    );
    assert!(
        visible.contains(&task1_id),
        "Task 1 should be visible when plan is expanded"
    );
    assert!(
        visible.contains(&task2_id),
        "Task 2 should be visible when plan is expanded"
    );
}
