//! Completion review cycle: routes task completion proposals to the user
//! and transitions work items based on the review answer.

use crate::graph::event::GraphEvent;
use crate::graph::node::{CompletionConfidence, QuestionDestination, QuestionStatus};
use crate::graph::{EdgeKind, Node};

use chrono::Utc;
use uuid::Uuid;

use super::App;

impl App {
    /// Handle a task agent proposing completion. Creates a review `Question` for
    /// the user to accept or reject. The question is linked to the work item via
    /// an `About` edge and routed through the standard `QuestionAdded` pipeline.
    pub(super) fn handle_completion_proposed(
        &mut self,
        work_item_id: Uuid,
        confidence: CompletionConfidence,
    ) {
        let title = {
            let g = self.graph.read();
            g.node(work_item_id)
                .map_or_else(|| "(unknown task)".to_string(), |n| n.content().to_string())
        };
        let question_content =
            format!("Task '{title}' completed with {confidence:?} confidence. Accept?");
        let question_id = Uuid::new_v4();
        let question_node = Node::Question {
            id: question_id,
            content: question_content,
            destination: QuestionDestination::User,
            status: QuestionStatus::Pending,
            requires_approval: true,
            created_at: Utc::now(),
        };
        let mut g = self.graph.write();
        g.add_node(question_node);
        let _ = g.add_edge(question_id, work_item_id, EdgeKind::About);
        g.emit(GraphEvent::QuestionAdded {
            node_id: question_id,
            destination: QuestionDestination::User,
        });
    }

    /// Handle a review answer for a completion question. If the question has an
    /// `About` edge pointing to a `WorkItem`, transition it to `Done`.
    /// If rejected (question goes back to `Pending`), the `WorkItem` stays `Active`,
    /// which fires `WorkItemStatusChanged { Active }` and spawns a new task agent.
    pub(super) fn handle_review_answer(&mut self, question_id: Uuid) {
        let g = self.graph.read();
        // Find the WorkItem this question is about (via About edge).
        let work_item_id = g
            .edges
            .iter()
            .find(|e| e.from == question_id && e.kind == EdgeKind::About)
            .map(|e| e.to);
        let Some(work_item_id) = work_item_id else {
            return; // Not a review question — no About edge to a WorkItem.
        };
        // Only act on WorkItem nodes.
        if !matches!(g.node(work_item_id), Some(Node::WorkItem { .. })) {
            return;
        }
        // Check if the question was answered (terminal state).
        let is_answered = matches!(
            g.node(question_id),
            Some(Node::Question {
                status: QuestionStatus::Answered,
                ..
            })
        );
        drop(g);

        if is_answered {
            let mut g = self.graph.write();
            if let Err(e) =
                g.update_work_item_status(work_item_id, crate::graph::WorkItemStatus::Done)
            {
                tracing::warn!("Failed to transition WorkItem {work_item_id} to Done: {e}");
            }
        }
        // If rejected → question goes back to Pending → WorkItem stays Active
        // → WorkItemStatusChanged { Active } fires → spawns new task agent.
    }
}
