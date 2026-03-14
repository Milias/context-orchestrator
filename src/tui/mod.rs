pub mod input;
pub mod state;
pub mod tabs;
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
pub const CURSOR_FRAMES: &[&str] = &["█", "▓", "▒", "░", "▒", "▓"];

#[derive(Debug)]
pub enum AgentVisualPhase {
    /// Pre-streaming: counting tokens, building context, connecting.
    Preparing,
    /// LLM is generating text.
    Streaming { text: String, is_thinking: bool },
    /// Tools are executing between iterations.
    ExecutingTools,
}

/// Display state for the entire agent run. Present when an agent loop is active.
#[derive(Debug)]
pub struct AgentDisplayState {
    pub phase: AgentVisualPhase,
    /// Assistant node IDs from this run (suppressed from history rendering).
    pub iteration_node_ids: Vec<Uuid>,
    pub spinner_tick: usize,
    /// How many characters of the streaming text are currently visible.
    /// Trails behind the actual text length to create a progressive reveal effect.
    pub revealed_chars: usize,
}

impl Default for AgentDisplayState {
    fn default() -> Self {
        Self {
            phase: AgentVisualPhase::Preparing,
            iteration_node_ids: Vec::new(),
            spinner_tick: 0,
            revealed_chars: 0,
        }
    }
}

impl AgentDisplayState {
    pub fn spinner_char(&self) -> &'static str {
        SPINNER_FRAMES[self.spinner_tick % SPINNER_FRAMES.len()]
    }

    /// Advance the character reveal toward the full text length.
    /// Called each spinner tick (80ms). During steady streaming (small deltas),
    /// reveals instantly. During bursts, spreads the reveal over ~400ms.
    pub fn advance_reveal(&mut self, total_chars: usize) {
        const BURST_THRESHOLD: usize = 15;
        const MIN_STEP: usize = 4;
        const CATCH_UP_FRAMES: usize = 5;

        let pending = total_chars.saturating_sub(self.revealed_chars);
        if pending == 0 {
            return;
        }

        let step = if pending <= BURST_THRESHOLD {
            pending
        } else {
            (pending / CATCH_UP_FRAMES).max(MIN_STEP)
        };
        self.revealed_chars = (self.revealed_chars + step).min(total_chars);
    }
}

// ── UI toggle enums (avoid bare bools — clippy::struct_excessive_bools) ──

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScrollMode {
    /// Automatically scroll to bottom on new content.
    Auto,
    /// User has manually scrolled; stay at current position.
    Manual,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolDisplayMode {
    /// Show compact status lines only (icon + name + duration).
    Compact,
    /// Show status lines with result content expanded below.
    Expanded,
}

impl ToolDisplayMode {
    pub fn toggle(self) -> Self {
        match self {
            Self::Compact => Self::Expanded,
            Self::Expanded => Self::Compact,
        }
    }

    pub fn is_expanded(self) -> bool {
        self == Self::Expanded
    }
}

// ── Token usage display ─────────────────────────────────────────────

/// A counter that animates toward a target value with ease-out motion.
///
/// Each [`tick`](Self::tick) advances `current` by 25% of the remaining
/// distance (minimum 1). A 10 000-token jump reaches its target in
/// roughly 15 ticks (~1.2 s at 80 ms per tick).
#[derive(Debug, Default, Clone, Copy)]
pub struct AnimatedCounter {
    /// The value currently shown in the UI.
    pub current: u64,
    /// The value we are animating toward.
    pub target: u64,
}

impl AnimatedCounter {
    /// Advance the displayed value one step toward the target.
    pub fn tick(&mut self) {
        if self.current < self.target {
            let step = ((self.target - self.current) / 4).max(1);
            self.current = (self.current + step).min(self.target);
        } else if self.current > self.target {
            // Snap immediately — decreasing totals are unexpected but must not
            // cause an infinite animation loop.
            self.current = self.target;
        }
    }

    /// Returns `true` while the displayed value differs from the target.
    pub fn is_animating(&self) -> bool {
        self.current != self.target
    }
}

/// Lifetime token totals displayed in the status bar.
#[derive(Debug, Default, Clone, Copy)]
pub struct TokenUsage {
    /// Input (prompt) tokens.
    pub input: AnimatedCounter,
    /// Output (completion) tokens.
    pub output: AnimatedCounter,
}

impl TokenUsage {
    /// Returns `true` while either counter is still animating.
    pub fn is_animating(&self) -> bool {
        self.input.is_animating() || self.output.is_animating()
    }

    /// Advance both counters one step.
    pub fn tick(&mut self) {
        self.input.tick();
        self.output.tick();
    }
}

// ── TUI state ────────────────────────────────────────────────────────

/// Central UI state shared across all rendering and input handling.
#[derive(Debug)]
pub struct TuiState {
    /// Top-level navigation: active tab, focus zone.
    pub nav: state::NavigationState,
    /// Current text in the input box.
    pub input_text: String,
    /// Character-indexed cursor position within `input_text`.
    pub input_cursor: usize,
    /// Conversation scroll offset (lines from the top).
    pub scroll_offset: u16,
    /// Informational status message shown in the status bar.
    pub status_message: Option<String>,
    /// Error message displayed right-aligned in red on the status bar.
    pub error_message: Option<String>,
    /// Set to `true` to exit the TUI event loop.
    pub should_quit: bool,
    /// Autoscroll vs. manual scroll state.
    pub scroll_mode: ScrollMode,
    /// Cached rendered markdown + height per message node ID.
    /// Avoids re-parsing markdown for historical messages on every frame.
    pub render_cache: HashMap<Uuid, CachedRender>,
    /// Autocomplete popup state for `/command` completion.
    pub autocomplete: AutocompleteState,
    /// Display state for the running agent loop. `None` when idle.
    pub agent_display: Option<AgentDisplayState>,
    /// Controls whether tool call results are shown inline in the conversation.
    pub tool_display: ToolDisplayMode,
    /// Maximum scroll offset, computed each frame by the conversation widget.
    /// Used by `handle_scroll` to clamp immediately (prevents over-scroll
    /// accumulation when the user scrolls rapidly past the content end).
    pub max_scroll: u16,
    /// Lifetime token usage displayed in the status bar (animated).
    pub token_usage: TokenUsage,
    /// Selected item index in the Work tab tree view.
    pub work_selected: usize,
    /// Number of visible items in the Work tab (set each frame by the renderer).
    pub work_visible_count: usize,
}

#[derive(Debug)]
pub struct CachedRender {
    pub styled_text: Text<'static>,
    pub height: usize,
    pub has_thinking: bool,
    pub cached_width: usize,
}

impl TuiState {
    /// Create a new TUI state with sensible defaults.
    pub fn new() -> Self {
        Self {
            nav: state::NavigationState::new(),
            input_text: String::new(),
            input_cursor: 0,
            scroll_offset: u16::MAX,
            status_message: None,
            error_message: None,
            should_quit: false,
            scroll_mode: ScrollMode::Auto,
            render_cache: HashMap::new(),
            autocomplete: AutocompleteState::default(),
            agent_display: None,
            tool_display: ToolDisplayMode::Compact,
            max_scroll: 0,
            token_usage: TokenUsage::default(),
            work_selected: 0,
            work_visible_count: 0,
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

#[cfg(test)]
#[path = "token_usage_tests.rs"]
mod token_usage_tests;

#[cfg(test)]
#[path = "reveal_tests.rs"]
mod reveal_tests;
