//! Navigation and view state for the tab-based TUI layout.
//!
//! `TopTab` controls which monitoring view fills the left content area.
//! `FocusZone` tracks which half of the screen owns keyboard focus.
//! Tab toggles between monitoring (left) and conversation+input (right).

/// Top-level tab controlling the left content area.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TopTab {
    /// Agent activity, attention items, stats. Default on startup.
    Agents,
    /// Plan/task tree with dependencies and detail.
    Work,
    /// Live event stream (tool calls, file changes, agent phases).
    Activity,
}

impl TopTab {
    /// All tabs in display order.
    pub fn all() -> &'static [TopTab] {
        &[TopTab::Agents, TopTab::Work, TopTab::Activity]
    }

    /// Display label for the tab bar.
    pub fn label(self) -> &'static str {
        match self {
            TopTab::Agents => "Agents",
            TopTab::Work => "Work",
            TopTab::Activity => "Activity",
        }
    }

    /// Look up a tab by its 1-indexed number key ('1' = Agents, etc.).
    pub fn from_number(n: u32) -> Option<TopTab> {
        match n {
            1 => Some(TopTab::Agents),
            2 => Some(TopTab::Work),
            3 => Some(TopTab::Activity),
            _ => None,
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
            active_tab: TopTab::Agents,
            focus: FocusZone::ChatPanel,
        }
    }

    /// Whether the right conversation panel should be rendered.
    /// True when the chat panel is focused.
    pub fn conversation_visible(&self) -> bool {
        self.focus == FocusZone::ChatPanel
    }
}
