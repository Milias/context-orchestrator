//! Per-section state for tree+detail navigation in the Graph tab.
//!
//! Each [`GraphSection`] gets its own `ExplorerState` tracking scroll
//! positions, selection, collapse set, and sub-panel focus. Switching
//! sections preserves each section's navigation state independently.

use std::collections::HashSet;
use uuid::Uuid;

use crate::tui::animated_scroll::AnimatedScroll;
use crate::tui::state::ExplorerFocus;

/// Per-section state for tree+detail navigation in the Graph tab.
///
/// Tracks scroll positions, selected item, collapsed nodes, and
/// which sub-panel (tree vs detail) has focus. One instance per
/// [`GraphSection`](crate::tui::state::GraphSection).
#[derive(Debug)]
pub struct ExplorerState {
    /// Animated scroll for the tree panel.
    pub tree_scroll: AnimatedScroll,
    /// Maximum scroll offset for tree panel (set each frame).
    pub tree_max: u16,
    /// Animated scroll for the detail panel.
    pub detail_scroll: AnimatedScroll,
    /// Maximum scroll offset for detail panel (set each frame).
    pub detail_max: u16,
    /// Index of the selected item in the flattened tree.
    pub selected: usize,
    /// Total visible items (set each frame by the renderer).
    pub visible_count: usize,
    /// Set of collapsed node UUIDs (expanded by default).
    pub collapsed: HashSet<Uuid>,
    /// Which sub-panel has focus: Tree or Detail.
    pub focus: ExplorerFocus,
}

impl ExplorerState {
    /// Create a new explorer state with everything at zero/default.
    pub fn new() -> Self {
        Self {
            tree_scroll: AnimatedScroll::zero(),
            tree_max: 0,
            detail_scroll: AnimatedScroll::zero(),
            detail_max: 0,
            selected: 0,
            visible_count: 0,
            collapsed: HashSet::new(),
            focus: ExplorerFocus::Tree,
        }
    }

    /// Toggle a node between collapsed and expanded.
    /// Collapsed nodes hide their children in the flattened tree.
    pub fn toggle_collapse(&mut self, id: Uuid) {
        if self.collapsed.contains(&id) {
            self.collapsed.remove(&id);
        } else {
            self.collapsed.insert(id);
        }
    }

    /// Check whether a node is currently collapsed.
    pub fn is_collapsed(&self, id: &Uuid) -> bool {
        self.collapsed.contains(id)
    }

    /// Move the selection up (negative) or down (positive).
    /// Clamps to `[0, visible_count - 1]` to prevent out-of-bounds.
    pub fn move_selection(&mut self, delta: isize) {
        if self.visible_count == 0 {
            self.selected = 0;
            return;
        }
        let max_index = self.visible_count - 1;
        let new_pos = isize::try_from(self.selected)
            .unwrap_or(0)
            .saturating_add(delta);
        self.selected = usize::try_from(new_pos.max(0)).unwrap_or(0).min(max_index);
    }

    /// Switch focus between Tree and Detail sub-panels.
    pub fn toggle_focus(&mut self) {
        self.focus = match self.focus {
            ExplorerFocus::Tree => ExplorerFocus::Detail,
            ExplorerFocus::Detail => ExplorerFocus::Tree,
        };
    }
}
