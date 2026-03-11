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
pub enum Focus {
    Input,
    BranchList,
}

#[derive(Debug)]
pub struct TuiState {
    pub focus: Focus,
    pub input_text: String,
    pub input_cursor: usize,
    pub scroll_offset: u16,
    pub branch_list_selected: usize,
    pub streaming_response: Option<String>,
    pub status_message: Option<String>,
    pub should_quit: bool,
    pub naming_branch: bool,
    pub branch_name_input: String,
}

impl TuiState {
    pub fn new() -> Self {
        Self {
            focus: Focus::Input,
            input_text: String::new(),
            input_cursor: 0,
            scroll_offset: 0,
            branch_list_selected: 0,
            streaming_response: None,
            status_message: None,
            should_quit: false,
            naming_branch: false,
            branch_name_input: String::new(),
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
