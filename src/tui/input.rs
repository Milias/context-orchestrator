use crate::tui::{FocusPanel, TuiState};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

#[derive(Debug)]
pub enum Action {
    None,
    Quit,
    SendMessage(String),
    ScrollUp,
    ScrollDown,
    PageUp,
    PageDown,
}

pub fn handle_key_event(key: KeyEvent, tui_state: &mut TuiState) -> Action {
    // Global keybindings
    if key.modifiers.contains(KeyModifiers::CONTROL) {
        match key.code {
            KeyCode::Char('q') => return Action::Quit,
            KeyCode::Char('b') => {
                tui_state.context_panel_visible = !tui_state.context_panel_visible;
                if !tui_state.context_panel_visible && tui_state.focus == FocusPanel::ContextPanel {
                    tui_state.focus = FocusPanel::Input;
                }
                return Action::None;
            }
            _ => {}
        }
    }

    if key.code == KeyCode::Tab {
        tui_state.focus = match tui_state.focus {
            FocusPanel::Input if tui_state.context_panel_visible => FocusPanel::ContextPanel,
            _ => FocusPanel::Input,
        };
        return Action::None;
    }

    match tui_state.focus {
        FocusPanel::Input => handle_input_key(key, tui_state),
        FocusPanel::ContextPanel => handle_context_panel_key(key, tui_state),
    }
}

fn handle_input_key(key: KeyEvent, tui_state: &mut TuiState) -> Action {
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
        KeyCode::Char(c) => {
            let byte_offset = tui_state
                .input_text
                .char_indices()
                .nth(tui_state.input_cursor)
                .map_or(tui_state.input_text.len(), |(i, _)| i);
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
                    .map_or(tui_state.input_text.len(), |(i, _)| i);
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
        KeyCode::PageUp => Action::PageUp,
        KeyCode::PageDown => Action::PageDown,
        _ => Action::None,
    }
}

fn handle_context_panel_key(key: KeyEvent, tui_state: &mut TuiState) -> Action {
    match key.code {
        KeyCode::Left => {
            tui_state.context_tab = tui_state.context_tab.prev();
            tui_state.context_list_offset = 0;
        }
        KeyCode::Right => {
            tui_state.context_tab = tui_state.context_tab.next();
            tui_state.context_list_offset = 0;
        }
        KeyCode::Up => {
            tui_state.context_list_offset = tui_state.context_list_offset.saturating_sub(1);
        }
        KeyCode::Down => {
            tui_state.context_list_offset += 1;
        }
        _ => {}
    }
    Action::None
}
