//! Tree connector system using box-drawing characters.
//!
//! Produces visual prefixes identical to the Unix `tree` command:
//!
//! ```text
//! ├── Plan: Refactor TUI [Active]
//! │   ├── Split overview tab
//! │   └── Add detail panel
//! └── Plan: API Integration [Active]
//!     └── Implement retry logic
//! ```
//!
//! Each depth level contributes exactly 4 characters of prefix width.

/// Vertical continuation connector: ancestor branch continues below.
const PIPE: &str = "│   ";

/// Empty space: ancestor branch has ended (was last sibling).
const SPACE: &str = "    ";

/// Branch connector: item has more siblings below it.
const TEE: &str = "├── ";

/// Corner connector: item is the last sibling at its level.
const CORNER: &str = "└── ";

/// Characters contributed by each depth level.
const CHARS_PER_DEPTH: usize = 4;

/// Accumulated tree-drawing state for rendering tree connectors.
///
/// Tracks which ancestor levels are "last sibling" to determine
/// whether to draw `│` (continuing) or ` ` (finished) at each depth.
#[derive(Clone, Debug)]
pub struct TreePrefix {
    /// Stack of `is_last_sibling` flags for each ancestor depth level.
    ///
    /// `true` means the ancestor at that level was the last sibling,
    /// so no vertical connector (`│`) should be drawn.
    ancestors: Vec<bool>,
}

impl TreePrefix {
    /// Create a new empty prefix (for root-level items).
    pub fn new() -> Self {
        Self {
            ancestors: Vec::new(),
        }
    }

    /// Create a child prefix by extending this one with a new depth level.
    ///
    /// `is_last` indicates whether the *current* item (the parent) is the
    /// last sibling at its level. The child needs this information to know
    /// whether the parent's branch continues or has ended.
    pub fn child(&self, is_last: bool) -> Self {
        let mut ancestors = self.ancestors.clone();
        ancestors.push(is_last);
        Self { ancestors }
    }

    /// Build the prefix string for a tree item.
    ///
    /// Returns the connector characters to prepend to the item's content.
    /// For root items (depth 0): just `├── ` or `└── `.
    /// For nested items: ancestor connectors followed by the item's own connector.
    ///
    /// `is_last_sibling`: whether this item is the last child of its parent.
    pub fn render(&self, is_last_sibling: bool) -> String {
        let mut buf = String::with_capacity(self.width() + CHARS_PER_DEPTH);

        // Draw ancestor continuation lines.
        for &ancestor_was_last in &self.ancestors {
            if ancestor_was_last {
                buf.push_str(SPACE);
            } else {
                buf.push_str(PIPE);
            }
        }

        // Draw the item's own connector.
        if is_last_sibling {
            buf.push_str(CORNER);
        } else {
            buf.push_str(TEE);
        }

        buf
    }

    /// Width in characters of the ancestor portion of this prefix.
    ///
    /// Does not include the item's own connector (add [`CHARS_PER_DEPTH`]
    /// for the total rendered width).
    fn width(&self) -> usize {
        self.ancestors.len() * CHARS_PER_DEPTH
    }

    /// Total width in characters of the prefix at the given depth.
    ///
    /// Each depth level contributes 4 characters, plus 4 for the item's
    /// own connector.
    pub fn rendered_width(depth: usize) -> usize {
        // `depth` ancestor columns + 1 own connector column.
        (depth + 1) * CHARS_PER_DEPTH
    }
}

impl Default for TreePrefix {
    fn default() -> Self {
        Self::new()
    }
}
