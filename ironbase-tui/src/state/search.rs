//! Search state for collections and document search

use super::types::SearchMode;

/// Search result - collection match
#[derive(Debug, Clone)]
pub struct SearchResult {
    pub name: String,
    pub doc_count: usize,
}

/// Document search match (line number where query was found)
#[derive(Debug, Clone)]
pub struct DocSearchMatch {
    pub line: usize,
    pub col_start: usize,
}

/// Search state
#[derive(Debug, Default)]
pub struct SearchState {
    pub query_input: String,
    pub cursor_pos: usize,
    pub input_active: bool,
    pub mode: SearchMode,
    // Collection search results
    pub results: Vec<SearchResult>,
    pub selected_result: usize,
    pub result_offset: usize,
    pub last_search: String,
    // Document search results
    pub doc_matches: Vec<DocSearchMatch>,
    pub current_match: usize,
}

impl SearchState {
    pub fn new() -> Self {
        Self {
            input_active: true,
            ..Default::default()
        }
    }

    pub fn clear(&mut self) {
        self.query_input.clear();
        self.cursor_pos = 0;
        self.results.clear();
        self.selected_result = 0;
        self.result_offset = 0;
        self.last_search.clear();
        self.doc_matches.clear();
        self.current_match = 0;
    }

    /// Clear only document search state (keep query)
    pub fn clear_doc_matches(&mut self) {
        self.doc_matches.clear();
        self.current_match = 0;
    }

    /// Navigate to next match
    pub fn next_match(&mut self) {
        if !self.doc_matches.is_empty() {
            self.current_match = (self.current_match + 1) % self.doc_matches.len();
        }
    }

    /// Navigate to previous match
    pub fn prev_match(&mut self) {
        if !self.doc_matches.is_empty() {
            self.current_match = if self.current_match == 0 {
                self.doc_matches.len() - 1
            } else {
                self.current_match - 1
            };
        }
    }

    /// Get current match line number (for scrolling)
    pub fn current_match_line(&self) -> Option<usize> {
        self.doc_matches.get(self.current_match).map(|m| m.line)
    }

    pub fn insert_char(&mut self, c: char) {
        // Convert character position to byte position for UTF-8 safety
        let byte_pos = self
            .query_input
            .char_indices()
            .nth(self.cursor_pos)
            .map(|(i, _)| i)
            .unwrap_or(self.query_input.len());
        self.query_input.insert(byte_pos, c);
        self.cursor_pos += 1;
    }

    pub fn delete_char(&mut self) {
        if self.cursor_pos > 0 {
            self.cursor_pos -= 1;
            // Convert character position to byte position for UTF-8 safety
            let byte_pos = self
                .query_input
                .char_indices()
                .nth(self.cursor_pos)
                .map(|(i, _)| i)
                .unwrap_or(0);
            self.query_input.remove(byte_pos);
        }
    }

    pub fn cursor_left(&mut self) {
        if self.cursor_pos > 0 {
            self.cursor_pos -= 1;
        }
    }

    pub fn cursor_right(&mut self) {
        // Use character count, not byte length
        if self.cursor_pos < self.query_input.chars().count() {
            self.cursor_pos += 1;
        }
    }
}
