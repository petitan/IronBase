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
            cursor: current_path.map(|p| p.chars().count()).unwrap_or(0),
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

    /// Convert char index to byte index for String operations
    /// (`cursor` is a char index, like in the renderer)
    fn char_to_byte_pos(&self, char_pos: usize) -> usize {
        self.path
            .char_indices()
            .nth(char_pos)
            .map(|(byte_pos, _)| byte_pos)
            .unwrap_or(self.path.len())
    }

    pub fn insert_char(&mut self, c: char) {
        let byte_pos = self.char_to_byte_pos(self.cursor);
        self.path.insert(byte_pos, c);
        self.cursor += 1;
        self.error = None;
    }

    pub fn backspace(&mut self) {
        if self.cursor > 0 {
            self.cursor -= 1;
            let byte_pos = self.char_to_byte_pos(self.cursor);
            self.path.remove(byte_pos);
            self.error = None;
        }
    }

    pub fn delete(&mut self) {
        if self.cursor < self.path.chars().count() {
            let byte_pos = self.char_to_byte_pos(self.cursor);
            self.path.remove(byte_pos);
            self.error = None;
        }
    }

    pub fn move_left(&mut self) {
        if self.cursor > 0 {
            self.cursor -= 1;
        }
    }

    pub fn move_right(&mut self) {
        if self.cursor < self.path.chars().count() {
            self.cursor += 1;
        }
    }

    pub fn home(&mut self) {
        self.cursor = 0;
    }

    pub fn end(&mut self) {
        self.cursor = self.path.chars().count();
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The cursor is a char index: typing after a multi-byte char must not
    /// hit a non-char-boundary (`String::insert` panicked on "é" + "x")
    #[test]
    fn test_path_input_handles_multibyte_chars() {
        let mut state = DatabaseState::new(Some("/tmp/á"), false);
        assert_eq!(state.cursor, 6);

        state.insert_char('é');
        state.insert_char('x');
        assert_eq!(state.path, "/tmp/áéx");
        assert_eq!(state.cursor, 8);

        state.move_left();
        state.move_left();
        state.insert_char('ő');
        assert_eq!(state.path, "/tmp/áőéx");

        state.delete();
        assert_eq!(state.path, "/tmp/áőx");
        state.backspace();
        assert_eq!(state.path, "/tmp/áx");

        state.end();
        assert_eq!(state.cursor, 7);
        state.move_right();
        assert_eq!(state.cursor, 7);
        state.backspace();
        state.backspace();
        assert_eq!(state.path, "/tmp/");
        state.delete();
        assert_eq!(state.path, "/tmp/");
    }

    #[test]
    fn test_new_collection_input_handles_multibyte_chars() {
        let mut state = NewCollectionState::new();
        state.insert_char('é');
        state.insert_char('x');
        state.cursor_left();
        state.insert_char('ő');
        assert_eq!(state.name, "éőx");
        state.backspace();
        assert_eq!(state.name, "éx");
    }
}
