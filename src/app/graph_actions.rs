//! Graph navigation action handlers for the app event loop.
//!
//! These methods resolve [`Action`] variants from the input handler
//! that require the conversation graph — e.g. collapse/expand
//! (needs UUID lookup), edge following (needs target resolution).

use crate::tui::state::ExplorerFocus;

use super::node_resolver::resolve_selected_node_id;
use super::App;

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
    /// (or a leaf), moves focus to the Detail sub-panel.
    pub(super) fn handle_graph_expand_or_focus(&mut self) {
        let section = self.tui_state.nav.active_graph_section;
        let graph = self.graph.read();
        let Some(node_id) = resolve_selected_node_id(&self.tui_state, section, &graph) else {
            return;
        };
        drop(graph);

        let Some(explorer) = self.tui_state.explorer.get_mut(&section) else {
            return;
        };

        if explorer.is_collapsed(&node_id) {
            explorer.collapsed.remove(&node_id);
        } else {
            explorer.focus = ExplorerFocus::Detail;
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
    /// backtrack, then resets edge selection for the target node.
    pub(super) fn handle_graph_follow_edge(&mut self) {
        let section = self.tui_state.nav.active_graph_section;

        // Verify that a valid edge is selected.
        let inspector = &self.tui_state.edge_inspector;
        if inspector.edges.get(inspector.selected_edge).is_none() {
            return;
        }

        // Resolve the current node for breadcrumb.
        let graph = self.graph.read();
        let current_node_id = resolve_selected_node_id(&self.tui_state, section, &graph);
        drop(graph);

        let Some(from_id) = current_node_id else {
            return;
        };

        // Push breadcrumb and reset edge selection.
        self.tui_state.edge_inspector.follow_edge(from_id);

        // Reset tree selection. The renderer will rebuild the flat list
        // and clamp the selection if needed. A future enhancement could
        // search the flat list for `target_id` and jump directly to it.
        if let Some(explorer) = self.tui_state.explorer.get_mut(&section) {
            explorer.selected = 0;
        }
    }

    /// Pop one breadcrumb from the edge inspector trail.
    ///
    /// Restores the previous node's edge selection index and returns
    /// focus to the tree if the trail is now empty.
    pub(super) fn handle_graph_pop_breadcrumb(&mut self) {
        let section = self.tui_state.nav.active_graph_section;

        let _prev_node = self.tui_state.edge_inspector.go_back();

        // If the trail is now empty, return focus to the tree.
        if self.tui_state.edge_inspector.trail.is_empty() {
            if let Some(explorer) = self.tui_state.explorer.get_mut(&section) {
                explorer.focus = ExplorerFocus::Tree;
            }
        }
    }
}
