//! Per-section state for tree+detail navigation in the Graph tab.
//!
//! Each [`GraphSection`] gets its own `ExplorerState` tracking scroll
//! positions, selection, collapse set, and sub-panel focus. Switching
//! sections preserves each section's navigation state independently.

use std::collections::HashSet;
use uuid::Uuid;

use crate::tui::state::ExplorerFocus;
use crate::tui::AnimatedScroll;

/// Per-section state for tree+detail navigation in the Graph tab.
///
/// Tracks scroll positions, selected item, collapsed nodes, and
/// which sub-panel (tree vs detail) has focus. One instance per
/// [`GraphSection`](crate::tui::state::GraphSection).
#[derive(Debug)]
pub struct ExplorerState {
    /// Index of the selected item in the flattened tree.
    pub selected: usize,
    /// Total visible items (set each frame by the renderer).
    pub visible_count: usize,
    /// Set of collapsed node UUIDs (expanded by default).
    pub collapsed: HashSet<Uuid>,
    /// Which sub-panel has focus: Tree or Detail.
    pub focus: ExplorerFocus,
    /// Animated scroll position for the tree viewport.
    pub scroll: AnimatedScroll,
}

impl ExplorerState {
    /// Create a new explorer state with everything at zero/default.
    pub fn new() -> Self {
        Self {
            selected: 0,
            visible_count: 0,
            collapsed: HashSet::new(),
            focus: ExplorerFocus::Tree,
            scroll: AnimatedScroll::zero(),
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

    /// After a mouse-driven scroll change, clamp selection to the visible range
    /// so the highlighted item stays within the viewport.
    pub fn clamp_selection_to_viewport(&mut self, viewport_height: usize) {
        let offset = self.scroll.position() as usize;
        let visible_end = offset + viewport_height;
        if self.selected < offset {
            self.selected = offset;
        } else if self.selected >= visible_end {
            self.selected = visible_end.saturating_sub(1);
        }
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
}
