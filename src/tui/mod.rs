pub mod input;
pub mod ui;
pub mod widgets;

use crossterm::{
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::prelude::*;
use std::io;

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
}

impl ContextTab {
    pub fn all() -> &'static [ContextTab] {
        &[
            ContextTab::Outline,
            ContextTab::Files,
            ContextTab::Tools,
            ContextTab::Tasks,
        ]
    }

    pub fn label(self) -> &'static str {
        match self {
            ContextTab::Outline => "Outline",
            ContextTab::Files => "Files",
            ContextTab::Tools => "Tools",
            ContextTab::Tasks => "Tasks",
        }
    }

    pub fn index(self) -> usize {
        match self {
            ContextTab::Outline => 0,
            ContextTab::Files => 1,
            ContextTab::Tools => 2,
            ContextTab::Tasks => 3,
        }
    }

    pub fn next(self) -> Self {
        match self {
            ContextTab::Outline => ContextTab::Files,
            ContextTab::Files => ContextTab::Tools,
            ContextTab::Tools => ContextTab::Tasks,
            ContextTab::Tasks => ContextTab::Outline,
        }
    }

    pub fn prev(self) -> Self {
        match self {
            ContextTab::Outline => ContextTab::Tasks,
            ContextTab::Files => ContextTab::Outline,
            ContextTab::Tools => ContextTab::Files,
            ContextTab::Tasks => ContextTab::Tools,
        }
    }
}

#[derive(Debug)]
pub struct TuiState {
    pub input_text: String,
    pub input_cursor: usize,
    pub scroll_offset: u16,
    pub streaming_response: Option<String>,
    pub status_message: Option<String>,
    pub should_quit: bool,
    pub focus: FocusPanel,
    pub context_panel_visible: bool,
    pub context_tab: ContextTab,
    pub context_list_offset: usize,
}

impl TuiState {
    pub fn new() -> Self {
        Self {
            input_text: String::new(),
            input_cursor: 0,
            scroll_offset: 0,
            streaming_response: None,
            status_message: None,
            should_quit: false,
            focus: FocusPanel::Input,
            context_panel_visible: true,
            context_tab: ContextTab::Outline,
            context_list_offset: 0,
        }
    }
}

pub fn setup_terminal() -> anyhow::Result<Terminal<CrosstermBackend<io::Stdout>>> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let terminal = Terminal::new(backend)?;
    Ok(terminal)
}

pub fn restore_terminal(
    mut terminal: Terminal<CrosstermBackend<io::Stdout>>,
) -> anyhow::Result<()> {
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    Ok(())
}
