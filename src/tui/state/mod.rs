//! Navigation and view state for the tab-based TUI layout.
//!
//! `TopTab` controls which monitoring view fills the left content area.
//! `FocusZone` tracks which half of the screen owns keyboard focus.
//! Tab toggles between monitoring (left) and conversation+input (right).

/// Top-level tab controlling the left content area.
///
/// `Overview` is the combined dashboard, `Graph` is the graph explorer,
/// and `System` surfaces background tasks and diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TopTab {
    /// Combined dashboard: agent card, running tasks, work tree, completions, stats.
    Overview,
    /// Graph explorer: tree+detail view with section tabs (Work, QA, Execution, Context).
    Graph,
    /// System diagnostics: background tasks, event log, resource usage.
    System,
}

impl TopTab {
    /// All tabs in display order.
    pub fn all() -> &'static [TopTab] {
        &[TopTab::Overview, TopTab::Graph, TopTab::System]
    }

    /// Display label for the tab bar.
    pub fn label(self) -> &'static str {
        match self {
            TopTab::Overview => "Overview",
            TopTab::Graph => "Graph",
            TopTab::System => "System",
        }
    }
}

/// Sub-section within the Graph tab.
///
/// Each section shows a different slice of the graph:
/// work items, QA flows, execution chains, or context operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum GraphSection {
    /// Work items: plans and tasks with their hierarchy.
    #[default]
    Work,
    /// Questions and answers with their routing/approval state.
    QA,
    /// Execution chains: messages, tool calls, tool results.
    Execution,
    /// Context building requests and their selected nodes.
    Context,
}

impl GraphSection {
    /// All sections in display order.
    pub fn all() -> &'static [GraphSection] {
        &[
            GraphSection::Work,
            GraphSection::QA,
            GraphSection::Execution,
            GraphSection::Context,
        ]
    }

    /// Display label for the section tab bar.
    pub fn label(self) -> &'static str {
        match self {
            GraphSection::Work => "Work",
            GraphSection::QA => "QA",
            GraphSection::Execution => "Execution",
            GraphSection::Context => "Context",
        }
    }
}

/// Which sub-panel within the Graph tab has keyboard focus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExplorerFocus {
    /// The left tree panel showing the node hierarchy.
    Tree,
    /// The right detail panel showing node properties and edges.
    Detail,
}

/// Which half of the screen owns keyboard focus.
/// Tab toggles between them; conversation panel visibility follows focus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusZone {
    /// Left side -- tab-specific monitoring/management panels.
    TabContent,
    /// Right side -- conversation + input (combined).
    /// Typing goes to input; Up/Down overflow scrolls conversation.
    ChatPanel,
}

/// Cached panel rectangles from the last render, used for mouse hit-testing.
/// Updated each frame by the rendering code.
#[derive(Debug, Default, Clone, Copy)]
pub struct PanelRects {
    /// Activity stream area in the overview tab.
    pub activity: ratatui::prelude::Rect,
    /// Recent completions area in the overview tab.
    pub recent: ratatui::prelude::Rect,
    /// Conversation panel area (right side).
    pub conversation: ratatui::prelude::Rect,
    /// Work tree area in the overview tab.
    pub work: ratatui::prelude::Rect,
    /// Tree panel area in the graph explorer tab.
    pub tree: ratatui::prelude::Rect,
    /// Detail panel area in the graph explorer tab.
    pub detail: ratatui::prelude::Rect,
}

/// Top-level navigation state for the tab-based layout.
#[derive(Debug)]
pub struct NavigationState {
    /// Which tab is active (controls the left content area).
    pub active_tab: TopTab,
    /// Which half of the screen owns keyboard focus.
    pub focus: FocusZone,
    /// Active section within the Graph tab.
    pub active_graph_section: GraphSection,
}

impl NavigationState {
    /// Create navigation state with sensible defaults.
    /// Starts focused on the chat panel (conversation visible).
    pub fn new() -> Self {
        Self {
            active_tab: TopTab::Overview,
            focus: FocusZone::ChatPanel,
            active_graph_section: GraphSection::default(),
        }
    }

    /// Whether the right conversation panel should be rendered.
    /// True when the chat panel is focused.
    pub fn conversation_visible(&self) -> bool {
        self.focus == FocusZone::ChatPanel
    }
}
