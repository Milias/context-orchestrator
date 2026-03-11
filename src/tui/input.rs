use crate::tui::TuiState;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

#[derive(Debug)]
pub enum Action {
    None,
    Quit,
    SendMessage(String),
    ScrollUp,
    ScrollDown,
}

pub fn handle_key_event(key: KeyEvent, tui_state: &mut TuiState) -> Action {
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('q') {
        return Action::Quit;
    }

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
