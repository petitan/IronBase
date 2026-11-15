//! Full-text search state

use crate::widgets::{InputOutcome, TextInput};
use crossterm::event::{KeyCode, KeyModifiers};
use ironbase_mcp_types::responses::FulltextHit;

/// Hits asked for per search (`fulltext_search` `limit`)
pub const FULLTEXT_LIMIT: usize = 50;

/// Full-text search state
#[derive(Debug, Clone, Default)]
pub struct FulltextState {
    pub collection: String,
    pub field: String,
    pub query: TextInput,
    /// Hits of the last search on `field`
    pub results: Vec<FulltextHit>,
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
            query: TextInput::default(),
            results: Vec::new(),
            selected_result: 0,
            error: None,
            loading: false,
            indexed_fields,
            selected_field_idx: 0,
        }
    }

    /// An editing key for the query; a change clears the error
    pub fn handle_input(&mut self, code: KeyCode, modifiers: KeyModifiers) -> InputOutcome {
        let outcome = self.query.handle_key(code, modifiers);
        if outcome == InputOutcome::Changed {
            self.error = None;
        }
        outcome
    }

    /// Switch to the next indexed field; when the field changes, the hits of
    /// the previous field's search are dropped (they do not belong to the new
    /// field). With a single indexed field nothing changes.
    pub fn next_field(&mut self) {
        if !self.indexed_fields.is_empty() {
            self.selected_field_idx = (self.selected_field_idx + 1) % self.indexed_fields.len();
            let field = &self.indexed_fields[self.selected_field_idx];
            if *field != self.field {
                self.field = field.clone();
                self.results.clear();
                self.selected_result = 0;
                self.error = None;
            }
        }
    }

    /// The last search returned a full page ([`FULLTEXT_LIMIT`] hits): more
    /// documents may match
    pub fn may_have_more(&self) -> bool {
        self.results.len() >= FULLTEXT_LIMIT
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
}
