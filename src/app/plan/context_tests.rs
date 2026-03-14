use crate::graph::{ConversationGraph, EdgeKind, Node, WorkItemKind, WorkItemStatus};

use chrono::Utc;
use uuid::Uuid;

/// Bug: flat rendering when `SubtaskOf` edges exist.
/// `build_plan_section` must show child tasks indented under their parent plan,
/// not as a flat list. If `SubtaskOf` edges are ignored, the children won't appear
/// in the output at all (they're Tasks, not Plans -- only Plans are top-level).
#[test]
fn test_build_plan_section_renders_hierarchy() {
    let mut graph = ConversationGraph::new("sys");

    // Create a Plan.
    let plan_id = Uuid::new_v4();
    graph.add_node(Node::WorkItem {
        id: plan_id,
        kind: WorkItemKind::Plan,
        title: "Refactor auth".to_string(),
        status: WorkItemStatus::Active,
        description: None,
        completion_confidence: None,
        created_at: Utc::now(),
    });

    // Create 2 child tasks with SubtaskOf edges.
    let task1_id = Uuid::new_v4();
    let task2_id = Uuid::new_v4();
    for (id, title) in [
        (task1_id, "Extract middleware"),
        (task2_id, "Add JWT support"),
    ] {
        graph.add_node(Node::WorkItem {
            id,
            kind: WorkItemKind::Task,
            title: title.to_string(),
            status: WorkItemStatus::Todo,
            description: None,
            completion_confidence: None,
        created_at: Utc::now(),
        });
        graph.add_edge(id, plan_id, EdgeKind::SubtaskOf).unwrap();
    }

    let section =
        super::build_plan_section(&graph).expect("should produce a section when plans exist");

    // The plan title must appear.
    assert!(
        section.contains("Refactor auth"),
        "output should contain plan title, got:\n{section}"
    );

    // Both child task titles must appear.
    assert!(
        section.contains("Extract middleware"),
        "output should contain first child task, got:\n{section}"
    );
    assert!(
        section.contains("Add JWT support"),
        "output should contain second child task, got:\n{section}"
    );

    // Children must be indented (rendered via `render_children` at depth 1).
    // Each child line should start with "  -" (2-space indent).
    let child_lines: Vec<&str> = section
        .lines()
        .filter(|l: &&str| l.contains("Extract middleware") || l.contains("Add JWT support"))
        .collect();
    assert_eq!(
        child_lines.len(),
        2,
        "both children should appear as separate lines"
    );
    for line in &child_lines {
        assert!(
            (*line).starts_with("  -"),
            "child lines should be indented with 2 spaces, got: {line}"
        );
    }
}

/// Bug: `build_plan_section` omits `depends on:` lines when `DependsOn` edges exist.
/// If the dependency rendering branch is dead or the edge query is broken, the output
/// will contain Plan A's title but never mention its prerequisite (Plan B).
#[test]
fn test_build_plan_section_renders_depends_on() {
    let mut graph = ConversationGraph::new("sys");

    // Plan B: the prerequisite.
    let prerequisite_id = Uuid::new_v4();
    graph.add_node(Node::WorkItem {
        id: prerequisite_id,
        kind: WorkItemKind::Plan,
        title: "Setup database".to_string(),
        status: WorkItemStatus::Active,
        description: None,
        completion_confidence: None,
        created_at: Utc::now(),
    });

    // Plan A: depends on Plan B.
    let dependent_id = Uuid::new_v4();
    graph.add_node(Node::WorkItem {
        id: dependent_id,
        kind: WorkItemKind::Plan,
        title: "Build API layer".to_string(),
        status: WorkItemStatus::Todo,
        description: None,
        completion_confidence: None,
        created_at: Utc::now(),
    });

    // Edge: A --DependsOn--> B.
    graph
        .add_edge(dependent_id, prerequisite_id, EdgeKind::DependsOn)
        .unwrap();

    let section =
        super::build_plan_section(&graph).expect("should produce a section when plans exist");

    // Plan A's title must appear.
    assert!(
        section.contains("Build API layer"),
        "output should contain Plan A's title, got:\n{section}"
    );

    // The dependency line must reference Plan B.
    assert!(
        section.contains("depends on:"),
        "output should contain a 'depends on:' line, got:\n{section}"
    );
    assert!(
        section.contains("Setup database"),
        "depends-on line should reference Plan B's title, got:\n{section}"
    );
}
