//! Full-text search state

use serde_json::Value;

/// Full-text search state
#[derive(Debug, Clone, Default)]
pub struct FulltextState {
    pub collection: String,
    pub field: String,
    pub query: String,
    pub cursor_pos: usize,
    pub results: Vec<Value>,
    pub selected_result: usize,
    pub error: Option<String>,
    pub loading: bool,
    /// Available fulltext-indexed fields
    pub indexed_fields: Vec<String>,
    pub selected_field_idx: usize,
}

impl FulltextState {
    pub fn new(collection: String, indexed_fields: Vec<String>) -> Self {
        let field = indexed_fields.first().cloned().unwrap_or_default();
        Self {
            collection,
            field,
            query: String::new(),
            cursor_pos: 0,
            results: Vec::new(),
            selected_result: 0,
            error: None,
            loading: false,
            indexed_fields,
            selected_field_idx: 0,
        }
    }

    /// Convert char index to byte index for String operations
    fn char_to_byte_pos(&self, char_pos: usize) -> usize {
        self.query
            .char_indices()
            .nth(char_pos)
            .map(|(byte_pos, _)| byte_pos)
            .unwrap_or(self.query.len())
    }

    pub fn insert_char(&mut self, c: char) {
        let byte_pos = self.char_to_byte_pos(self.cursor_pos);
        self.query.insert(byte_pos, c);
        self.cursor_pos += 1;
        self.error = None;
    }

    pub fn backspace(&mut self) {
        if self.cursor_pos > 0 {
            self.cursor_pos -= 1;
            let byte_pos = self.char_to_byte_pos(self.cursor_pos);
            self.query.remove(byte_pos);
            self.error = None;
        }
    }

    pub fn delete(&mut self) {
        if self.cursor_pos < self.query.chars().count() {
            let byte_pos = self.char_to_byte_pos(self.cursor_pos);
            self.query.remove(byte_pos);
            self.error = None;
        }
    }

    pub fn move_cursor_left(&mut self) {
        if self.cursor_pos > 0 {
            self.cursor_pos -= 1;
        }
    }

    pub fn move_cursor_right(&mut self) {
        if self.cursor_pos < self.query.chars().count() {
            self.cursor_pos += 1;
        }
    }

    pub fn next_field(&mut self) {
        if !self.indexed_fields.is_empty() {
            self.selected_field_idx = (self.selected_field_idx + 1) % self.indexed_fields.len();
            self.field = self.indexed_fields[self.selected_field_idx].clone();
        }
    }

    pub fn select_up(&mut self) {
        if self.selected_result > 0 {
            self.selected_result -= 1;
        }
    }

    pub fn select_down(&mut self) {
        if self.selected_result + 1 < self.results.len() {
            self.selected_result += 1;
        }
    }

    pub fn clear_results(&mut self) {
        self.results.clear();
        self.selected_result = 0;
    }
}
