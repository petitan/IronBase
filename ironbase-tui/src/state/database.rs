//! Database and collection state

use super::types::DatabaseMode;

/// Database open/create state
#[derive(Debug, Clone, Default)]
pub struct DatabaseState {
    pub path: String,
    pub cursor: usize,
    pub error: Option<String>,
    pub message: Option<String>,
    pub loading: bool,
    pub is_http_mode: bool,
    pub mode: DatabaseMode,
}

impl DatabaseState {
    pub fn new(current_path: Option<&str>, is_http: bool) -> Self {
        Self {
            path: current_path.unwrap_or("").to_string(),
            cursor: current_path.map(|p| p.len()).unwrap_or(0),
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

    pub fn insert_char(&mut self, c: char) {
        self.path.insert(self.cursor, c);
        self.cursor += 1;
        self.error = None;
    }

    pub fn backspace(&mut self) {
        if self.cursor > 0 {
            self.cursor -= 1;
            self.path.remove(self.cursor);
            self.error = None;
        }
    }

    pub fn delete(&mut self) {
        if self.cursor < self.path.len() {
            self.path.remove(self.cursor);
            self.error = None;
        }
    }

    pub fn move_left(&mut self) {
        if self.cursor > 0 {
            self.cursor -= 1;
        }
    }

    pub fn move_right(&mut self) {
        if self.cursor < self.path.len() {
            self.cursor += 1;
        }
    }

    pub fn home(&mut self) {
        self.cursor = 0;
    }

    pub fn end(&mut self) {
        self.cursor = self.path.len();
    }
}

/// New collection state
#[derive(Debug, Clone, Default)]
pub struct NewCollectionState {
    pub name: String,
    pub cursor: usize,
    pub error: Option<String>,
}

impl NewCollectionState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert_char(&mut self, c: char) {
        // Only allow valid collection name characters
        if c.is_alphanumeric() || c == '_' || c == '-' {
            let byte_pos = self
                .name
                .char_indices()
                .nth(self.cursor)
                .map(|(i, _)| i)
                .unwrap_or(self.name.len());
            self.name.insert(byte_pos, c);
            self.cursor += 1;
            self.error = None;
        }
    }

    pub fn backspace(&mut self) {
        if self.cursor > 0 {
            let byte_pos = self
                .name
                .char_indices()
                .nth(self.cursor - 1)
                .map(|(i, _)| i)
                .unwrap_or(0);
            self.name.remove(byte_pos);
            self.cursor -= 1;
            self.error = None;
        }
    }

    pub fn cursor_left(&mut self) {
        if self.cursor > 0 {
            self.cursor -= 1;
        }
    }

    pub fn cursor_right(&mut self) {
        if self.cursor < self.name.chars().count() {
            self.cursor += 1;
        }
    }
}
