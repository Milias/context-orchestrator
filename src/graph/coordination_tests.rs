use crate::graph::node::{QuestionDestination, QuestionStatus};
use crate::graph::{ConversationGraph, EdgeKind, Node, TaskStatus, WorkItemKind, WorkItemStatus};
use chrono::Utc;
use uuid::Uuid;

/// Helper: create a `WorkItem` node.
fn add_work_item(graph: &mut ConversationGraph, status: WorkItemStatus) -> Uuid {
    let id = Uuid::new_v4();
    graph.add_node(Node::WorkItem {
        id,
        kind: WorkItemKind::Task,
        title: "test task".to_string(),
        status,
        description: None,
        created_at: Utc::now(),
    });
    id
}

/// Helper: create a Question node.
fn add_question(graph: &mut ConversationGraph, status: QuestionStatus) -> Uuid {
    let id = Uuid::new_v4();
    graph.add_node(Node::Question {
        id,
        content: "test question".to_string(),
        destination: QuestionDestination::User,
        status,
        requires_approval: false,
        created_at: Utc::now(),
    });
    id
}

/// Helper: create a `BackgroundTask` node.
fn add_bg_task(graph: &mut ConversationGraph, status: TaskStatus) -> Uuid {
    use crate::graph::BackgroundTaskKind;
    let id = Uuid::new_v4();
    graph.add_node(Node::BackgroundTask {
        id,
        kind: BackgroundTaskKind::AgentPhase,
        status,
        description: "test task".to_string(),
        created_at: Utc::now(),
        updated_at: Utc::now(),
    });
    id
}

/// Bug: `is_resolved` returns true for a non-Done `WorkItem`, allowing
/// dependents to start prematurely.
#[test]
fn test_is_resolved_work_item_status() {
    let mut graph = ConversationGraph::new("system");

    let todo = add_work_item(&mut graph, WorkItemStatus::Todo);
    let active = add_work_item(&mut graph, WorkItemStatus::Active);
    let done = add_work_item(&mut graph, WorkItemStatus::Done);

    assert!(!graph.is_resolved(todo), "Todo should not be resolved");
    assert!(!graph.is_resolved(active), "Active should not be resolved");
    assert!(graph.is_resolved(done), "Done should be resolved");
}

/// Bug: `is_resolved` returns true for non-Answered `Question`.
#[test]
fn test_is_resolved_question_status() {
    let mut graph = ConversationGraph::new("system");

    let pending = add_question(&mut graph, QuestionStatus::Pending);
    let claimed = add_question(&mut graph, QuestionStatus::Claimed);
    let answered = add_question(&mut graph, QuestionStatus::Answered);

    assert!(
        !graph.is_resolved(pending),
        "Pending should not be resolved"
    );
    assert!(
        !graph.is_resolved(claimed),
        "Claimed should not be resolved"
    );
    assert!(graph.is_resolved(answered), "Answered should be resolved");
}

/// Bug: `is_resolved` returns true for non-Completed `BackgroundTask`.
#[test]
fn test_is_resolved_background_task_status() {
    let mut graph = ConversationGraph::new("system");

    let running = add_bg_task(&mut graph, TaskStatus::Running);
    let completed = add_bg_task(&mut graph, TaskStatus::Completed);
    let failed = add_bg_task(&mut graph, TaskStatus::Failed);

    assert!(
        !graph.is_resolved(running),
        "Running should not be resolved"
    );
    assert!(graph.is_resolved(completed), "Completed should be resolved");
    assert!(!graph.is_resolved(failed), "Failed should not be resolved");
}

/// Bug: `is_resolved` returns true for a non-existent node,
/// unblocking dependents of deleted nodes.
#[test]
fn test_is_resolved_nonexistent_returns_false() {
    let graph = ConversationGraph::new("system");
    assert!(
        !graph.is_resolved(Uuid::new_v4()),
        "nonexistent node should not be resolved"
    );
}

/// Bug: `ready_unclaimed_nodes` includes nodes with zero dependencies.
/// Only nodes that HAD blocking deps (now resolved) should appear.
#[test]
fn test_ready_unclaimed_excludes_zero_dep_nodes() {
    let mut graph = ConversationGraph::new("system");

    // Node with no dependencies — should NOT be in the ready set.
    let no_deps = add_work_item(&mut graph, WorkItemStatus::Todo);

    let ready = graph.ready_unclaimed_nodes();
    assert!(
        !ready.contains(&no_deps),
        "node with zero deps should be excluded"
    );
}

/// Bug: `ready_unclaimed_nodes` includes nodes whose deps are not all resolved.
#[test]
fn test_ready_unclaimed_excludes_unresolved_deps() {
    let mut graph = ConversationGraph::new("system");

    let dep = add_work_item(&mut graph, WorkItemStatus::Active); // not done
    let blocked = add_work_item(&mut graph, WorkItemStatus::Todo);
    let _ = graph.add_edge(blocked, dep, EdgeKind::DependsOn);

    assert!(
        !graph.ready_unclaimed_nodes().contains(&blocked),
        "should be blocked by Active dependency"
    );
}

/// Bug: `release_claim` does not remove the `ClaimedBy` edge — node
/// stays claimed forever, preventing re-assignment.
#[test]
fn test_release_claim_makes_unclaimed() {
    let mut graph = ConversationGraph::new("system");
    let node = add_work_item(&mut graph, WorkItemStatus::Todo);
    let agent = Uuid::new_v4();

    graph.try_claim(node, agent);
    assert!(graph.is_claimed(node));

    graph.release_claim(node);
    assert!(!graph.is_claimed(node), "should be unclaimed after release");
}

/// Bug: `ready_unclaimed_nodes` includes claimed nodes — allows
/// double-execution by multiple agents.
#[test]
fn test_ready_unclaimed_excludes_claimed() {
    let mut graph = ConversationGraph::new("system");

    let dep = add_work_item(&mut graph, WorkItemStatus::Done);
    let task = add_work_item(&mut graph, WorkItemStatus::Todo);
    let _ = graph.add_edge(task, dep, EdgeKind::DependsOn);

    let agent = Uuid::new_v4();
    graph.try_claim(task, agent);

    assert!(
        !graph.ready_unclaimed_nodes().contains(&task),
        "claimed node should not appear in ready set"
    );
}
