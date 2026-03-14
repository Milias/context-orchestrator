//! Graph navigation action handlers for the app event loop.
//!
//! These methods resolve [`Action`] variants from the input handler
//! that require the conversation graph — e.g. collapse/expand
//! (needs UUID lookup), edge following (needs target resolution).

use uuid::Uuid;

use crate::graph::ConversationGraph;
use crate::tui::state::ExplorerFocus;
use crate::tui::tabs::edge_inspector::{DisplayEdge, EdgeInspector};
use crate::tui::widgets::tool_status::truncate;

use super::node_resolver::resolve_selected_node_id;
use super::App;

/// Maximum length for the one-line target summary in edge display.
const EDGE_SUMMARY_MAX_LEN: usize = 40;

/// Populate the edge inspector with edges for the given node.
///
/// Clears any previous edges, resolves each edge's target summary from
/// the graph, and resets the selection index. This ensures the input
/// handler and the detail-panel renderer both operate on the same list.
fn populate_edges(graph: &ConversationGraph, inspector: &mut EdgeInspector, node_id: Uuid) {
    inspector.edges.clear();
    inspector.selected_edge = 0;

    for (_direction, kind, other_id) in graph.edges_of(node_id) {
        let target_summary = graph.node(other_id).map_or_else(
            || "(unknown)".to_string(),
            |n| {
                truncate(
                    n.content().lines().next().unwrap_or(""),
                    EDGE_SUMMARY_MAX_LEN,
                )
            },
        );

        inspector.edges.push(DisplayEdge {
            group: kind.group(),
            label: kind.display_label(),
            target_summary,
            target_id: other_id,
        });
    }
}

impl App {
    /// Toggle collapse/expand on the node currently selected in the tree.
    ///
    /// Looks up the selected node's UUID via the section's tree builder
    /// (which mirrors the renderer's flattening logic), then delegates
    /// to [`ExplorerState::toggle_collapse`].
    pub(super) fn handle_graph_toggle_collapse(&mut self) {
        let section = self.tui_state.nav.active_graph_section;
        let graph = self.graph.read();
        let Some(node_id) = resolve_selected_node_id(&self.tui_state, section, &graph) else {
            return;
        };
        drop(graph);

        if let Some(explorer) = self.tui_state.explorer.get_mut(&section) {
            explorer.toggle_collapse(node_id);
        }
    }

    /// Expand a collapsed node, or shift focus to the detail panel.
    ///
    /// If the selected node is collapsed, expands it. If already expanded
    /// (or a leaf), moves focus to the Detail sub-panel and populates the
    /// edge inspector for the current node.
    pub(super) fn handle_graph_expand_or_focus(&mut self) {
        let section = self.tui_state.nav.active_graph_section;
        let graph = self.graph.read();
        let Some(node_id) = resolve_selected_node_id(&self.tui_state, section, &graph) else {
            return;
        };

        let Some(explorer) = self.tui_state.explorer.get_mut(&section) else {
            return;
        };

        if explorer.is_collapsed(&node_id) {
            explorer.collapsed.remove(&node_id);
        } else {
            explorer.focus = ExplorerFocus::Detail;
            populate_edges(&graph, &mut self.tui_state.edge_inspector, node_id);
        }
    }

    /// Collapse the selected node if it is currently expanded.
    ///
    /// If the node is already collapsed (or a leaf), this is a no-op.
    pub(super) fn handle_graph_collapse_node(&mut self) {
        let section = self.tui_state.nav.active_graph_section;
        let graph = self.graph.read();
        let Some(node_id) = resolve_selected_node_id(&self.tui_state, section, &graph) else {
            return;
        };
        drop(graph);

        if let Some(explorer) = self.tui_state.explorer.get_mut(&section) {
            if !explorer.is_collapsed(&node_id) {
                explorer.collapsed.insert(node_id);
            }
        }
    }

    /// Follow the selected edge in the detail panel.
    ///
    /// Records a breadcrumb for the current node so the user can
    /// backtrack, then populates edges for the target node.
    pub(super) fn handle_graph_follow_edge(&mut self) {
        let section = self.tui_state.nav.active_graph_section;

        // Verify that a valid edge is selected and capture the target.
        let target_id = {
            let inspector = &self.tui_state.edge_inspector;
            let Some(edge) = inspector.edges.get(inspector.selected_edge) else {
                return;
            };
            edge.target_id
        };

        // Resolve the current node for breadcrumb.
        let graph = self.graph.read();
        let current_node_id = resolve_selected_node_id(&self.tui_state, section, &graph);

        let Some(from_id) = current_node_id else {
            return;
        };

        // Push breadcrumb and reset edge selection.
        self.tui_state.edge_inspector.follow_edge(from_id);

        // Populate edges for the newly focused target node.
        populate_edges(&graph, &mut self.tui_state.edge_inspector, target_id);

        // Reset tree selection. The renderer will rebuild the flat list
        // and clamp the selection if needed. A future enhancement could
        // search the flat list for `target_id` and jump directly to it.
        if let Some(explorer) = self.tui_state.explorer.get_mut(&section) {
            explorer.selected = 0;
        }
    }

    /// Pop one breadcrumb from the edge inspector trail.
    ///
    /// Restores the previous node's edge selection index, re-populates
    /// edges for the restored node, and returns focus to the tree if
    /// the trail is now empty.
    pub(super) fn handle_graph_pop_breadcrumb(&mut self) {
        let section = self.tui_state.nav.active_graph_section;

        let Some(restored_node) = self.tui_state.edge_inspector.go_back() else {
            return;
        };

        // Re-populate edges for the node we're returning to.
        // Save and restore `selected_edge` because `go_back()` already
        // set it to the breadcrumb's stored index.
        let saved_edge = self.tui_state.edge_inspector.selected_edge;
        let graph = self.graph.read();
        populate_edges(&graph, &mut self.tui_state.edge_inspector, restored_node);
        // Restore the previously selected edge, clamped to bounds.
        let edge_count = self.tui_state.edge_inspector.edges.len();
        self.tui_state.edge_inspector.selected_edge = saved_edge.min(edge_count.saturating_sub(1));

        // If the trail is now empty, return focus to the tree.
        if self.tui_state.edge_inspector.trail.is_empty() {
            if let Some(explorer) = self.tui_state.explorer.get_mut(&section) {
                explorer.focus = ExplorerFocus::Tree;
            }
        }
    }
}
