//! Database and collection state

use super::types::DatabaseMode;
use crate::widgets::{Input, InputOutcome, TextInput};
use crossterm::event::KeyCode;

/// Database open/create state
#[derive(Debug, Clone, Default)]
pub struct DatabaseState {
    pub path: TextInput,
    pub error: Option<String>,
    pub message: Option<String>,
    pub loading: bool,
    pub is_http_mode: bool,
    pub mode: DatabaseMode,
}

impl DatabaseState {
    pub fn new(current_path: Option<&str>, is_http: bool) -> Self {
        Self {
            path: TextInput::new(current_path.unwrap_or("")),
            error: None,
            message: None,
            loading: false,
            is_http_mode: is_http,
            mode: DatabaseMode::Open,
        }
    }

    pub fn toggle_mode(&mut self) {
        self.mode = match self.mode {
            DatabaseMode::Open => DatabaseMode::Create,
            DatabaseMode::Create => DatabaseMode::Open,
        };
        self.error = None;
    }

    /// An editing key for the path; a change clears the error
    pub fn handle_input(&mut self, input: Input) -> InputOutcome {
        let outcome = self.path.apply(input);
        if outcome == InputOutcome::Changed {
            self.error = None;
        }
        outcome
    }
}

/// New collection state
#[derive(Debug, Clone, Default)]
pub struct NewCollectionState {
    pub name: TextInput,
    pub error: Option<String>,
}

impl NewCollectionState {
    pub fn new() -> Self {
        Self::default()
    }

    /// An editing key for the name: only the characters of a valid
    /// collection name are typed; a change clears the error
    pub fn handle_input(&mut self, input: Input) -> InputOutcome {
        let valid = |c: char| c.is_alphanumeric() || c == '_' || c == '-';
        // A pasted name keeps only those characters
        let pasted: String;
        let input = match input {
            Input::Key(KeyCode::Char(c), _) if !valid(c) => return InputOutcome::Ignored,
            Input::Paste(text) => {
                pasted = text.chars().filter(|&c| valid(c)).collect();
                Input::Paste(&pasted)
            }
            key => key,
        };
        let outcome = self.name.apply(input);
        if outcome == InputOutcome::Changed {
            self.error = None;
        }
        outcome
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyModifiers;

    #[test]
    fn test_path_input_starts_at_the_end_and_clears_the_error() {
        let mut state = DatabaseState::new(Some("/tmp/á"), false);
        assert_eq!(state.path.cursor(), 6);
        state.error = Some("old".to_string());

        state.handle_input(Input::Key(KeyCode::Left, KeyModifiers::NONE));
        assert!(state.error.is_some());
        state.handle_input(Input::Key(KeyCode::Char('é'), KeyModifiers::NONE));
        assert_eq!(state.path.value(), "/tmp/éá");
        assert_eq!(state.error, None);
    }

    #[test]
    fn test_new_collection_name_takes_only_name_characters() {
        let mut state = NewCollectionState::new();
        for c in ['é', 'x', ' ', '.', '/', '_', '-', '1'] {
            state.handle_input(Input::Key(KeyCode::Char(c), KeyModifiers::NONE));
        }
        assert_eq!(state.name.value(), "éx_-1");
        state.handle_input(Input::Key(KeyCode::Char('w'), KeyModifiers::CONTROL));
        assert!(state.name.is_empty());
    }
}
