//! Navigation and view state for the tab-based TUI layout.
//!
//! `TopTab` controls which monitoring view fills the left content area.
//! `FocusZone` tracks which half of the screen owns keyboard focus.
//! Tab toggles between monitoring (left) and conversation+input (right).

/// Top-level tab controlling the left content area.
///
/// Currently a single `Overview` tab combines agents, work, and activity.
/// The tab container is preserved for future expansion.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TopTab {
    /// Combined dashboard: agent card, running tasks, work tree, completions, stats.
    Overview,
}

impl TopTab {
    /// All tabs in display order.
    pub fn all() -> &'static [TopTab] {
        &[TopTab::Overview]
    }

    /// Display label for the tab bar.
    pub fn label(self) -> &'static str {
        match self {
            TopTab::Overview => "Overview",
        }
    }
}

/// Which half of the screen owns keyboard focus.
/// Tab toggles between them; conversation panel visibility follows focus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusZone {
    /// Left side — tab-specific monitoring/management panels.
    TabContent,
    /// Right side — conversation + input (combined).
    /// Typing goes to input; Up/Down overflow scrolls conversation.
    ChatPanel,
}

/// Cached panel rectangles from the last render, used for mouse hit-testing.
/// Updated each frame by the rendering code.
#[derive(Debug, Default, Clone, Copy)]
pub struct PanelRects {
    /// Activity stream area in the overview tab.
    pub activity: ratatui::prelude::Rect,
    /// Conversation panel area (right side).
    pub conversation: ratatui::prelude::Rect,
    /// Work tree area in the overview tab.
    pub work: ratatui::prelude::Rect,
}

/// Top-level navigation state for the tab-based layout.
#[derive(Debug)]
pub struct NavigationState {
    /// Which tab is active (controls the left content area).
    pub active_tab: TopTab,
    /// Which half of the screen owns keyboard focus.
    pub focus: FocusZone,
}

impl NavigationState {
    /// Create navigation state with sensible defaults.
    /// Starts focused on the chat panel (conversation visible).
    pub fn new() -> Self {
        Self {
            active_tab: TopTab::Overview,
            focus: FocusZone::ChatPanel,
        }
    }

    /// Whether the right conversation panel should be rendered.
    /// True when the chat panel is focused.
    pub fn conversation_visible(&self) -> bool {
        self.focus == FocusZone::ChatPanel
    }
}
