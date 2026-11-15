//! Search state for collections and document search

use super::types::SearchMode;
use crate::widgets::TextInput;

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
}

/// Search state
#[derive(Debug, Default)]
pub struct SearchState {
    pub query_input: TextInput,
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
}
