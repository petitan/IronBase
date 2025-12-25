//! Insert/Edit document state

use super::types::EditorMode;
use serde_json::Value;

/// Document editor state - multi-line JSON editor for insert/edit
#[derive(Debug, Clone)]
pub struct InsertState {
    pub lines: Vec<String>,
    pub cursor_line: usize,
    pub cursor_col: usize,
    pub collection: String,
    pub error: Option<String>,
    pub mode: EditorMode,
    pub original_doc_id: Option<Value>,
}

impl Default for InsertState {
    fn default() -> Self {
        Self {
            lines: vec!["{".to_string(), "  ".to_string(), "}".to_string()],
            cursor_line: 1,
            cursor_col: 2,
            collection: String::new(),
            error: None,
            mode: EditorMode::Insert,
            original_doc_id: None,
        }
    }
}

impl InsertState {
    pub fn new(collection: String) -> Self {
        Self {
            collection,
            mode: EditorMode::Insert,
            ..Default::default()
        }
    }

    /// Create insert state with template (from existing document or default)
    pub fn with_template(collection: String, template: Option<Value>) -> Self {
        let lines = if let Some(val) = template {
            let json_str = serde_json::to_string_pretty(&val).unwrap_or_else(|_| "{}".to_string());
            json_str.lines().map(String::from).collect()
        } else {
            // Default template for empty collection
            vec![
                "{".to_string(),
                "  \"name\": \"\",".to_string(),
                "  \"value\": \"\"".to_string(),
                "}".to_string(),
            ]
        };

        Self {
            lines,
            cursor_line: 1,
            cursor_col: 2,
            collection,
            error: None,
            mode: EditorMode::Insert,
            original_doc_id: None,
        }
    }

    /// Create editor state for editing an existing document
    pub fn edit(collection: String, doc: &Value) -> Self {
        let json_str = serde_json::to_string_pretty(doc).unwrap_or_else(|_| "{}".to_string());
        let lines: Vec<String> = json_str.lines().map(String::from).collect();
        let doc_id = doc.get("_id").cloned();

        Self {
            lines,
            cursor_line: 1,
            cursor_col: 0,
            collection,
            error: None,
            mode: EditorMode::Edit,
            original_doc_id: doc_id,
        }
    }

    pub fn clear(&mut self) {
        self.lines = vec!["{".to_string(), "  ".to_string(), "}".to_string()];
        self.cursor_line = 1;
        self.cursor_col = 2;
        self.error = None;
        self.mode = EditorMode::Insert;
        self.original_doc_id = None;
    }

    pub fn is_edit_mode(&self) -> bool {
        self.mode == EditorMode::Edit
    }

    /// Insert a character at cursor position
    pub fn insert_char(&mut self, c: char) {
        if let Some(line) = self.lines.get_mut(self.cursor_line) {
            let char_count = line.chars().count();
            if self.cursor_col <= char_count {
                // Convert character position to byte position for UTF-8 safety
                let byte_pos = line
                    .char_indices()
                    .nth(self.cursor_col)
                    .map(|(i, _)| i)
                    .unwrap_or(line.len());
                line.insert(byte_pos, c);
                self.cursor_col += 1;
            }
        }
        self.error = None;
    }

    /// Delete character before cursor (backspace)
    pub fn backspace(&mut self) {
        if self.cursor_col > 0 {
            if let Some(line) = self.lines.get_mut(self.cursor_line) {
                // Convert character position to byte position for UTF-8 safety
                let byte_pos = line
                    .char_indices()
                    .nth(self.cursor_col - 1)
                    .map(|(i, _)| i)
                    .unwrap_or(0);
                line.remove(byte_pos);
                self.cursor_col -= 1;
            }
        } else if self.cursor_line > 0 {
            // Join with previous line
            let current_line = self.lines.remove(self.cursor_line);
            self.cursor_line -= 1;
            if let Some(prev_line) = self.lines.get_mut(self.cursor_line) {
                self.cursor_col = prev_line.chars().count();
                prev_line.push_str(&current_line);
            }
        }
        self.error = None;
    }

    /// Insert a new line at cursor
    pub fn insert_newline(&mut self) {
        if let Some(line) = self.lines.get_mut(self.cursor_line) {
            // Convert character position to byte position for UTF-8 safety
            let byte_pos = line
                .char_indices()
                .nth(self.cursor_col)
                .map(|(i, _)| i)
                .unwrap_or(line.len());
            let rest = line.split_off(byte_pos);
            self.cursor_line += 1;
            self.lines.insert(self.cursor_line, rest);
            self.cursor_col = 0;
        }
        self.error = None;
    }

    /// Insert 2 spaces (Tab key)
    pub fn insert_tab(&mut self) {
        self.insert_char(' ');
        self.insert_char(' ');
    }

    /// Move cursor left
    pub fn cursor_left(&mut self) {
        if self.cursor_col > 0 {
            self.cursor_col -= 1;
        } else if self.cursor_line > 0 {
            self.cursor_line -= 1;
            // Use character count, not byte length
            self.cursor_col = self
                .lines
                .get(self.cursor_line)
                .map(|l| l.chars().count())
                .unwrap_or(0);
        }
    }

    /// Move cursor right
    pub fn cursor_right(&mut self) {
        // Use character count, not byte length
        let char_count = self
            .lines
            .get(self.cursor_line)
            .map(|l| l.chars().count())
            .unwrap_or(0);
        if self.cursor_col < char_count {
            self.cursor_col += 1;
        } else if self.cursor_line + 1 < self.lines.len() {
            self.cursor_line += 1;
            self.cursor_col = 0;
        }
    }

    /// Move cursor up
    pub fn cursor_up(&mut self) {
        if self.cursor_line > 0 {
            self.cursor_line -= 1;
            // Use character count, not byte length
            let char_count = self
                .lines
                .get(self.cursor_line)
                .map(|l| l.chars().count())
                .unwrap_or(0);
            self.cursor_col = self.cursor_col.min(char_count);
        }
    }

    /// Move cursor down
    pub fn cursor_down(&mut self) {
        if self.cursor_line + 1 < self.lines.len() {
            self.cursor_line += 1;
            // Use character count, not byte length
            let char_count = self
                .lines
                .get(self.cursor_line)
                .map(|l| l.chars().count())
                .unwrap_or(0);
            self.cursor_col = self.cursor_col.min(char_count);
        }
    }

    /// Get the full JSON text
    pub fn get_json_text(&self) -> String {
        self.lines.join("\n")
    }

    /// Validate and parse JSON
    pub fn parse_json(&self) -> Result<serde_json::Value, String> {
        let text = self.get_json_text();
        serde_json::from_str(&text).map_err(|e| e.to_string())
    }
}
