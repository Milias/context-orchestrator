//! Edge inspector state for navigating between nodes via their edges.
//!
//! The inspector shows outgoing/incoming edges grouped by kind, and
//! supports following an edge to its target (pushing a breadcrumb)
//! and going back through the trail.

use uuid::Uuid;

use crate::graph::EdgeGroup;

/// A single edge prepared for display in the inspector panel.
#[derive(Debug, Clone)]
pub struct DisplayEdge {
    /// Visual group this edge belongs to.
    pub group: EdgeGroup,
    /// Human-readable label for the edge kind.
    pub label: &'static str,
    /// One-line summary of the target node's content.
    pub target_summary: String,
    /// UUID of the node this edge points to.
    pub target_id: Uuid,
}

/// A breadcrumb entry recording a navigation step through edges.
#[derive(Debug, Clone)]
pub struct Breadcrumb {
    /// The node we navigated away from.
    pub node_id: Uuid,
    /// Which edge index was selected when we followed the edge.
    pub edge_index: usize,
}

/// Maximum breadcrumb depth before the oldest entries are dropped.
const MAX_TRAIL_DEPTH: usize = 10;

/// State for the edge inspector panel.
///
/// Shows edges of the currently selected node, supports following
/// an edge to its target (pushing a breadcrumb), and backtracking.
#[derive(Debug)]
pub struct EdgeInspector {
    /// Edges of the currently inspected node, ready for display.
    pub edges: Vec<DisplayEdge>,
    /// Index of the currently highlighted edge.
    pub selected_edge: usize,
    /// Breadcrumb trail for back-navigation.
    pub trail: Vec<Breadcrumb>,
}

impl EdgeInspector {
    /// Create an empty edge inspector with no edges or trail.
    pub fn new() -> Self {
        Self {
            edges: Vec::new(),
            selected_edge: 0,
            trail: Vec::new(),
        }
    }

    /// Follow an edge: record the current node in the breadcrumb trail
    /// so we can return to it later. The `node_id` is the node we are
    /// leaving; `selected_edge` is saved automatically.
    ///
    /// The trail is capped at [`MAX_TRAIL_DEPTH`] entries; the oldest
    /// breadcrumb is dropped when the limit is exceeded.
    pub fn follow_edge(&mut self, node_id: Uuid) {
        self.trail.push(Breadcrumb {
            node_id,
            edge_index: self.selected_edge,
        });
        if self.trail.len() > MAX_TRAIL_DEPTH {
            self.trail.remove(0);
        }
        self.selected_edge = 0;
    }

    /// Go back one step in the breadcrumb trail.
    /// Returns the node UUID to navigate back to, or `None` if at the root.
    pub fn go_back(&mut self) -> Option<Uuid> {
        let crumb = self.trail.pop()?;
        self.selected_edge = crumb.edge_index;
        Some(crumb.node_id)
    }

    /// Clear all edges and the breadcrumb trail.
    /// Used when switching to a different node via tree selection.
    pub fn clear(&mut self) {
        self.edges.clear();
        self.selected_edge = 0;
        self.trail.clear();
    }
}
