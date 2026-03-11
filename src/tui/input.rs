use crate::tui::{Focus, TuiState};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

#[derive(Debug)]
pub enum Action {
    None,
    Quit,
    SendMessage(String),
    CreateBranch(String),
    SwitchBranch(usize),
    ToggleFocus,
    ScrollUp,
    ScrollDown,
}

pub fn handle_key_event(key: KeyEvent, tui_state: &mut TuiState) -> Action {
    // Ctrl+Q always quits
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('q') {
        return Action::Quit;
    }

    if tui_state.naming_branch {
        return handle_branch_naming(key, tui_state);
    }

    match tui_state.focus {
        Focus::Input => handle_input_mode(key, tui_state),
        Focus::BranchList => handle_branch_list_mode(key, tui_state),
    }
}

fn handle_input_mode(key: KeyEvent, tui_state: &mut TuiState) -> Action {
    match key.code {
        KeyCode::Enter => {
            let text = tui_state.input_text.trim().to_string();
            if text.is_empty() {
                return Action::None;
            }
            tui_state.input_text.clear();
            tui_state.input_cursor = 0;
            Action::SendMessage(text)
        }
        KeyCode::Char('b') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            tui_state.naming_branch = true;
            tui_state.branch_name_input.clear();
            Action::None
        }
        KeyCode::Tab => Action::ToggleFocus,
        KeyCode::Char(c) => {
            // Convert cursor (char count) to byte offset
            let byte_offset = tui_state
                .input_text
                .char_indices()
                .nth(tui_state.input_cursor)
                .map(|(i, _)| i)
                .unwrap_or(tui_state.input_text.len());
            tui_state.input_text.insert(byte_offset, c);
            tui_state.input_cursor += 1;
            Action::None
        }
        KeyCode::Backspace => {
            if tui_state.input_cursor > 0 {
                tui_state.input_cursor -= 1;
                let byte_offset = tui_state
                    .input_text
                    .char_indices()
                    .nth(tui_state.input_cursor)
                    .map(|(i, _)| i)
                    .unwrap_or(tui_state.input_text.len());
                tui_state.input_text.remove(byte_offset);
            }
            Action::None
        }
        KeyCode::Left => {
            if tui_state.input_cursor > 0 {
                tui_state.input_cursor -= 1;
            }
            Action::None
        }
        KeyCode::Right => {
            if tui_state.input_cursor < tui_state.input_text.chars().count() {
                tui_state.input_cursor += 1;
            }
            Action::None
        }
        KeyCode::Up => Action::ScrollUp,
        KeyCode::Down => Action::ScrollDown,
        _ => Action::None,
    }
}

fn handle_branch_list_mode(key: KeyEvent, tui_state: &mut TuiState) -> Action {
    match key.code {
        KeyCode::Up => {
            if tui_state.branch_list_selected > 0 {
                tui_state.branch_list_selected -= 1;
            }
            Action::SwitchBranch(tui_state.branch_list_selected)
        }
        KeyCode::Down => {
            tui_state.branch_list_selected += 1; // clamped by app.rs
            Action::SwitchBranch(tui_state.branch_list_selected)
        }
        KeyCode::Tab => Action::ToggleFocus,
        KeyCode::Enter => Action::SwitchBranch(tui_state.branch_list_selected),
        _ => Action::None,
    }
}

fn handle_branch_naming(key: KeyEvent, tui_state: &mut TuiState) -> Action {
    match key.code {
        KeyCode::Enter => {
            let name = tui_state.branch_name_input.trim().to_string();
            tui_state.naming_branch = false;
            tui_state.branch_name_input.clear();
            if name.is_empty() {
                Action::None
            } else {
                Action::CreateBranch(name)
            }
        }
        KeyCode::Esc => {
            tui_state.naming_branch = false;
            tui_state.branch_name_input.clear();
            Action::None
        }
        KeyCode::Char(c) => {
            tui_state.branch_name_input.push(c);
            Action::None
        }
        KeyCode::Backspace => {
            tui_state.branch_name_input.pop();
            Action::None
        }
        _ => Action::None,
    }
}
