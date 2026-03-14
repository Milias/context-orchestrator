//! Navigation and view state for the tab-based TUI layout.
//!
//! `TopTab` controls which monitoring view fills the left content area.
//! `FocusZone` tracks which region owns keyboard focus.
//! The conversation panel is a persistent right-side panel, not a tab.

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

/// Which region of the screen owns keyboard focus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusZone {
    /// Left side — tab-specific monitoring/management panels.
    TabContent,
    /// Right side — persistent conversation panel.
    Conversation,
    /// Bottom — message input box.
    Input,
}

impl FocusZone {
    /// Cycle to the next zone. Skips `Conversation` when the panel is hidden.
    pub fn next(self, conversation_visible: bool) -> Self {
        match self {
            FocusZone::TabContent if conversation_visible => FocusZone::Conversation,
            FocusZone::TabContent | FocusZone::Conversation => FocusZone::Input,
            FocusZone::Input => FocusZone::TabContent,
        }
    }
}

/// Top-level navigation state for the tab-based layout.
#[derive(Debug)]
pub struct NavigationState {
    /// Which tab is active (controls the left content area).
    pub active_tab: TopTab,
    /// Which screen region owns keyboard focus.
    pub focus: FocusZone,
    /// Whether the right conversation panel is visible.
    pub conversation_visible: bool,
}

impl NavigationState {
    /// Create navigation state with sensible defaults.
    pub fn new() -> Self {
        Self {
            active_tab: TopTab::Agents,
            focus: FocusZone::Input,
            conversation_visible: true,
        }
    }
}
