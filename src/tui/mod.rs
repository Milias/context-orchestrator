pub mod input;
pub mod ui;
pub mod widgets;

use crossterm::{
    event::{KeyboardEnhancementFlags, PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags},
    execute,
    terminal::{
        disable_raw_mode, enable_raw_mode, supports_keyboard_enhancement, EnterAlternateScreen,
        LeaveAlternateScreen,
    },
};
use ratatui::prelude::*;
use std::collections::HashMap;
use std::io;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusPanel {
    Input,
    ContextPanel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContextTab {
    Outline,
    Files,
    Tools,
    Tasks,
    Work,
}

impl ContextTab {
    pub fn all() -> &'static [ContextTab] {
        &[
            ContextTab::Outline,
            ContextTab::Files,
            ContextTab::Tools,
            ContextTab::Tasks,
            ContextTab::Work,
        ]
    }

    pub fn label(self) -> &'static str {
        match self {
            ContextTab::Outline => "Outline",
            ContextTab::Files => "Files",
            ContextTab::Tools => "Tools",
            ContextTab::Tasks => "Tasks",
            ContextTab::Work => "Work",
        }
    }

    pub fn index(self) -> usize {
        match self {
            ContextTab::Outline => 0,
            ContextTab::Files => 1,
            ContextTab::Tools => 2,
            ContextTab::Tasks => 3,
            ContextTab::Work => 4,
        }
    }

    pub fn next(self) -> Self {
        match self {
            ContextTab::Outline => ContextTab::Files,
            ContextTab::Files => ContextTab::Tools,
            ContextTab::Tools => ContextTab::Tasks,
            ContextTab::Tasks => ContextTab::Work,
            ContextTab::Work => ContextTab::Outline,
        }
    }

    pub fn prev(self) -> Self {
        match self {
            ContextTab::Outline => ContextTab::Work,
            ContextTab::Files => ContextTab::Outline,
            ContextTab::Tools => ContextTab::Files,
            ContextTab::Tasks => ContextTab::Tools,
            ContextTab::Work => ContextTab::Tasks,
        }
    }
}

#[derive(Debug, Clone)]
pub struct CompletionCandidate {
    pub name: String,
    pub description: String,
}

#[derive(Debug, Default)]
pub struct AutocompleteState {
    pub active: bool,
    pub trigger_char: char,
    pub prefix: String,
    pub candidates: Vec<CompletionCandidate>,
    pub selected: usize,
}

// ── Agent display state ──────────────────────────────────────────────

pub const SPINNER_FRAMES: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

#[derive(Debug)]
pub enum AgentVisualPhase {
    /// Pre-streaming: counting tokens, building context, connecting.
    Preparing,
    /// LLM is generating text.
    Streaming { text: String, is_thinking: bool },
    /// Tools are executing between iterations.
    ExecutingTools { tool_count: usize },
}

/// Display state for the entire agent run. Present when an agent loop is active.
#[derive(Debug)]
pub struct AgentDisplayState {
    pub phase: AgentVisualPhase,
    /// Text accumulated across all completed iterations.
    pub accumulated_text: String,
    /// Assistant node IDs from this run (suppressed from history rendering).
    pub iteration_node_ids: Vec<Uuid>,
    pub spinner_tick: usize,
}

impl AgentDisplayState {
    pub fn spinner_char(&self) -> &'static str {
        SPINNER_FRAMES[self.spinner_tick % SPINNER_FRAMES.len()]
    }
}

// ── TUI state ────────────────────────────────────────────────────────

#[derive(Debug)]
pub struct TuiState {
    pub input_text: String,
    pub input_cursor: usize,
    pub scroll_offset: u16,
    pub status_message: Option<String>,
    /// Error message displayed right-aligned in red on the status bar.
    pub error_message: Option<String>,
    pub should_quit: bool,
    pub focus: FocusPanel,
    pub context_panel_visible: bool,
    pub context_tab: ContextTab,
    pub context_list_offset: usize,
    /// When true, new streaming chunks auto-scroll to the bottom.
    /// Set to false when the user manually scrolls during streaming.
    pub auto_scroll: bool,
    /// Cached rendered markdown + height per message node ID.
    /// Avoids re-parsing markdown for historical messages on every frame.
    pub render_cache: HashMap<Uuid, CachedRender>,
    pub autocomplete: AutocompleteState,
    pub available_tools: Vec<CompletionCandidate>,
    /// Display state for the running agent loop. `None` when idle.
    pub agent_display: Option<AgentDisplayState>,
}

#[derive(Debug)]
pub struct CachedRender {
    pub styled_text: Text<'static>,
    pub height: usize,
    pub has_thinking: bool,
    pub cached_width: usize,
}

impl TuiState {
    pub fn new() -> Self {
        Self {
            input_text: String::new(),
            input_cursor: 0,
            scroll_offset: u16::MAX,
            status_message: None,
            error_message: None,
            should_quit: false,
            focus: FocusPanel::Input,
            context_panel_visible: true,
            context_tab: ContextTab::Outline,
            context_list_offset: 0,
            auto_scroll: true,
            render_cache: HashMap::new(),
            autocomplete: AutocompleteState::default(),
            available_tools: Vec::new(),
            agent_display: None,
        }
    }
}

pub fn setup_terminal() -> anyhow::Result<Terminal<CrosstermBackend<io::Stdout>>> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    // Push AFTER entering alternate screen — Kitty clears the keyboard
    // enhancement stack on screen switch, so pushing before would be lost.
    if supports_keyboard_enhancement().unwrap_or(false) {
        execute!(
            stdout,
            PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
        )?;
    }
    let backend = CrosstermBackend::new(stdout);
    let terminal = Terminal::new(backend)?;
    Ok(terminal)
}

pub fn restore_terminal(
    mut terminal: Terminal<CrosstermBackend<io::Stdout>>,
) -> anyhow::Result<()> {
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        PopKeyboardEnhancementFlags,
        LeaveAlternateScreen
    )?;
    terminal.show_cursor()?;
    Ok(())
}
