//! Application state and navigation - Pane-based architecture

use crate::config::Config;
use crate::db::{CollectionInfo, DbWrapper};
use crate::modals::confirm::{ConfirmAction, ConfirmOption, ConfirmState};
use crate::modals::export::ExportFormat;
use crate::theme::{Theme, ThemeName};
use serde_json::Value;
use std::path::PathBuf;

// === UI Layout Constants ===

/// Terminal overhead (header + command bar + borders)
const TERMINAL_OVERHEAD: u16 = 4;

/// Minimum document page size
const MIN_PAGE_SIZE: u16 = 5;

/// Detail pane scroll step (also used for page jump in collections)
const SCROLL_STEP: usize = 10;

/// Active pane in the 3-panel layout
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Pane {
    #[default]
    Collections,
    Documents,
    Detail,
}

impl Pane {
    /// Get next pane (Tab)
    pub fn next(&self) -> Self {
        match self {
            Pane::Collections => Pane::Documents,
            Pane::Documents => Pane::Detail,
            Pane::Detail => Pane::Collections,
        }
    }

    /// Get previous pane (Shift+Tab)
    pub fn prev(&self) -> Self {
        match self {
            Pane::Collections => Pane::Detail,
            Pane::Documents => Pane::Collections,
            Pane::Detail => Pane::Documents,
        }
    }
}

/// Currently active modal (if any)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Modal {
    Search,
    Actions,
    Help,
    Confirm,
    Insert,
    Index,
    Query,
    Export,
    Filter,
    ErrorDetail,
    NewCollection,
    Script,
    ServerInfo,
    Update,
    Database,
    ApiKey,
}
/// Search mode - collections or document content
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SearchMode {
    #[default]
    Collections,
    Document,
}

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

/// Filter operator for visual search
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FilterOperator {
    #[default]
    Equals, // $eq
    NotEquals,   // $ne
    GreaterThan, // $gt
    LessThan,    // $lt
    GreaterOrEq, // $gte
    LessOrEq,    // $lte
    Contains,    // $regex (case insensitive)
    StartsWith,  // $regex ^
    Exists,      // $exists
    In,          // $in (comma separated)
}

impl FilterOperator {
    pub fn all() -> &'static [FilterOperator] {
        &[
            Self::Equals,
            Self::NotEquals,
            Self::Contains,
            Self::StartsWith,
            Self::GreaterThan,
            Self::GreaterOrEq,
            Self::LessThan,
            Self::LessOrEq,
            Self::Exists,
            Self::In,
        ]
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::Equals => "=",
            Self::NotEquals => "≠",
            Self::GreaterThan => ">",
            Self::LessThan => "<",
            Self::GreaterOrEq => ">=",
            Self::LessOrEq => "<=",
            Self::Contains => "contains",
            Self::StartsWith => "starts with",
            Self::Exists => "exists",
            Self::In => "in list",
        }
    }

    pub fn to_query(&self, field: &str, value: &str) -> Value {
        use serde_json::json;

        // Helper to parse value as number or return string
        let parse_value = |v: &str| -> Value {
            if let Ok(n) = v.parse::<i64>() {
                json!(n)
            } else if let Ok(n) = v.parse::<f64>() {
                json!(n)
            } else if v == "true" {
                json!(true)
            } else if v == "false" {
                json!(false)
            } else if v == "null" {
                json!(null)
            } else {
                json!(v)
            }
        };

        match self {
            Self::Equals => {
                let v = parse_value(value);
                json!({ (field): v })
            }
            Self::NotEquals => {
                let v = parse_value(value);
                json!({ (field): { "$ne": v } })
            }
            Self::GreaterThan => {
                let v = parse_value(value);
                json!({ (field): { "$gt": v } })
            }
            Self::LessThan => {
                let v = parse_value(value);
                json!({ (field): { "$lt": v } })
            }
            Self::GreaterOrEq => {
                let v = parse_value(value);
                json!({ (field): { "$gte": v } })
            }
            Self::LessOrEq => {
                let v = parse_value(value);
                json!({ (field): { "$lte": v } })
            }
            Self::Contains => json!({ (field): { "$regex": value } }),
            Self::StartsWith => json!({ (field): { "$regex": format!("^{}", value) } }),
            Self::Exists => json!({ (field): { "$exists": value != "false" && value != "0" } }),
            Self::In => {
                let values: Vec<Value> = value
                    .split(',')
                    .map(|s| s.trim())
                    .filter(|s| !s.is_empty()) // Skip empty values
                    .map(parse_value)
                    .collect();
                if values.is_empty() {
                    // Return query that matches nothing if no valid values
                    json!({ (field): { "$in": [] } })
                } else {
                    json!({ (field): { "$in": values } })
                }
            }
        }
    }
}

/// A single filter condition
#[derive(Debug, Clone)]
pub struct FilterCondition {
    pub field: String,
    pub operator: FilterOperator,
    pub value: String,
}

impl FilterCondition {
    pub fn to_query(&self) -> Value {
        self.operator.to_query(&self.field, &self.value)
    }

    pub fn display(&self) -> String {
        if self.operator == FilterOperator::Exists {
            format!("{} {}", self.field, self.operator.label())
        } else {
            format!(
                "{} {} \"{}\"",
                self.field,
                self.operator.label(),
                self.value
            )
        }
    }
}

/// Focus state for filter modal
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FilterFocus {
    #[default]
    Field,
    Operator,
    Value,
    Filters,
    SortField,
}

/// Sort direction
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SortDirection {
    #[default]
    Asc,
    Desc,
}

impl SortDirection {
    pub fn toggle(&self) -> Self {
        match self {
            SortDirection::Asc => SortDirection::Desc,
            SortDirection::Desc => SortDirection::Asc,
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            SortDirection::Asc => "↑ Növekvő",
            SortDirection::Desc => "↓ Csökkenő",
        }
    }

    pub fn to_value(&self) -> i32 {
        match self {
            SortDirection::Asc => 1,
            SortDirection::Desc => -1,
        }
    }
}

/// Visual filter state
#[derive(Debug, Clone, Default)]
pub struct FilterState {
    pub field_input: String,
    pub field_cursor: usize,
    pub operator: FilterOperator,
    pub operator_idx: usize,
    pub value_input: String,
    pub value_cursor: usize,
    pub focus: FilterFocus,
    pub filters: Vec<FilterCondition>,
    pub selected_filter: usize,
    /// All available field names from collection schema
    pub all_fields: Vec<String>,
    /// Filtered suggestions based on current input
    pub filtered_suggestions: Vec<String>,
    pub suggestion_idx: usize,
    pub show_suggestions: bool,
    pub results: Option<Vec<Value>>,
    pub result_count: usize,
    pub error: Option<String>,
    // === Sort ===
    pub sort_field: Option<String>,
    pub sort_direction: SortDirection,
    pub sort_field_idx: usize,
}

impl FilterState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Reset input fields only, preserving existing filters and sort
    pub fn reset_inputs(&mut self) {
        self.field_input.clear();
        self.field_cursor = 0;
        self.value_input.clear();
        self.value_cursor = 0;
        self.operator = FilterOperator::default();
        self.operator_idx = 0;
        self.focus = FilterFocus::Field;
        // Preserve: filters, selected_filter, all_fields, sort_field, sort_direction
        self.filtered_suggestions.clear();
        self.suggestion_idx = 0;
        self.show_suggestions = false;
        // Preserve results for context, clear error
        self.error = None;
        // Update suggestions based on schema
        self.update_suggestions();
    }

    /// Build sort object for query
    pub fn build_sort(&self) -> Option<Value> {
        self.sort_field
            .as_ref()
            .map(|field| serde_json::json!({ field: self.sort_direction.to_value() }))
    }

    /// Select next sort field from available fields
    pub fn next_sort_field(&mut self) {
        if self.all_fields.is_empty() {
            return;
        }
        self.sort_field_idx = (self.sort_field_idx + 1) % (self.all_fields.len() + 1);
        self.sort_field = if self.sort_field_idx == 0 {
            None // First option = no sort
        } else {
            Some(self.all_fields[self.sort_field_idx - 1].clone())
        };
    }

    /// Select previous sort field
    pub fn prev_sort_field(&mut self) {
        if self.all_fields.is_empty() {
            return;
        }
        let total = self.all_fields.len() + 1;
        self.sort_field_idx = if self.sort_field_idx == 0 {
            total - 1
        } else {
            self.sort_field_idx - 1
        };
        self.sort_field = if self.sort_field_idx == 0 {
            None
        } else {
            Some(self.all_fields[self.sort_field_idx - 1].clone())
        };
    }

    /// Toggle sort direction
    pub fn toggle_sort_direction(&mut self) {
        self.sort_direction = self.sort_direction.toggle();
    }

    /// Clear sort
    pub fn clear_sort(&mut self) {
        self.sort_field = None;
        self.sort_field_idx = 0;
        self.sort_direction = SortDirection::Asc;
    }

    pub fn clear(&mut self) {
        self.field_input.clear();
        self.field_cursor = 0;
        self.value_input.clear();
        self.value_cursor = 0;
        self.operator = FilterOperator::default();
        self.operator_idx = 0;
        self.filters.clear();
        self.selected_filter = 0;
        // Keep all_fields - they're from the collection schema
        self.filtered_suggestions.clear();
        self.suggestion_idx = 0;
        self.show_suggestions = false;
        self.results = None;
        self.result_count = 0;
        self.error = None;
        // Clear sort too
        self.sort_field = None;
        self.sort_field_idx = 0;
        self.sort_direction = SortDirection::Asc;
    }

    pub fn next_focus(&mut self) {
        self.focus = match self.focus {
            FilterFocus::Field => {
                self.show_suggestions = false;
                FilterFocus::Operator
            }
            FilterFocus::Operator => FilterFocus::Value,
            FilterFocus::Value => {
                if self.filters.is_empty() {
                    FilterFocus::SortField
                } else {
                    FilterFocus::Filters
                }
            }
            FilterFocus::Filters => FilterFocus::SortField,
            FilterFocus::SortField => {
                self.update_suggestions();
                FilterFocus::Field
            }
        };
    }

    pub fn prev_focus(&mut self) {
        self.focus = match self.focus {
            FilterFocus::Field => {
                self.show_suggestions = false;
                FilterFocus::SortField
            }
            FilterFocus::Operator => {
                self.update_suggestions();
                FilterFocus::Field
            }
            FilterFocus::Value => FilterFocus::Operator,
            FilterFocus::Filters => FilterFocus::Value,
            FilterFocus::SortField => {
                if self.filters.is_empty() {
                    FilterFocus::Value
                } else {
                    FilterFocus::Filters
                }
            }
        };
    }

    pub fn next_operator(&mut self) {
        let ops = FilterOperator::all();
        self.operator_idx = (self.operator_idx + 1) % ops.len();
        self.operator = ops[self.operator_idx];
    }

    pub fn prev_operator(&mut self) {
        let ops = FilterOperator::all();
        if self.operator_idx == 0 {
            self.operator_idx = ops.len() - 1;
        } else {
            self.operator_idx -= 1;
        }
        self.operator = ops[self.operator_idx];
    }

    pub fn add_filter(&mut self) {
        if !self.field_input.is_empty() {
            // Use mem::take to avoid clone+clear pattern
            let condition = FilterCondition {
                field: std::mem::take(&mut self.field_input),
                operator: self.operator,
                value: std::mem::take(&mut self.value_input),
            };
            self.filters.push(condition);
            self.field_cursor = 0;
            self.value_cursor = 0;
            self.operator = FilterOperator::default();
            self.operator_idx = 0;
            self.focus = FilterFocus::Field;
        }
    }

    pub fn remove_selected_filter(&mut self) {
        if !self.filters.is_empty() {
            self.filters.remove(self.selected_filter);
            if self.selected_filter >= self.filters.len() && !self.filters.is_empty() {
                self.selected_filter = self.filters.len() - 1;
            }
            if self.filters.is_empty() {
                self.focus = FilterFocus::Field;
            }
        }
    }

    /// Load selected filter into input fields for editing
    pub fn edit_selected_filter(&mut self) {
        if let Some(filter) = self.filters.get(self.selected_filter).cloned() {
            // Load filter values into input fields
            self.field_input = filter.field;
            self.field_cursor = self.field_input.chars().count();
            self.value_input = filter.value;
            self.value_cursor = self.value_input.chars().count();
            self.operator = filter.operator;
            self.operator_idx = FilterOperator::all()
                .iter()
                .position(|&op| op == filter.operator)
                .unwrap_or(0);

            // Remove the filter being edited
            self.filters.remove(self.selected_filter);
            if self.selected_filter >= self.filters.len() && !self.filters.is_empty() {
                self.selected_filter = self.filters.len() - 1;
            }

            // Move focus to field for editing
            self.focus = FilterFocus::Field;
            self.show_suggestions = false;
        }
    }

    /// Move selected filter up in the list
    pub fn move_filter_up(&mut self) {
        if self.selected_filter > 0 && !self.filters.is_empty() {
            self.filters
                .swap(self.selected_filter, self.selected_filter - 1);
            self.selected_filter -= 1;
        }
    }

    /// Move selected filter down in the list
    pub fn move_filter_down(&mut self) {
        if self.selected_filter + 1 < self.filters.len() {
            self.filters
                .swap(self.selected_filter, self.selected_filter + 1);
            self.selected_filter += 1;
        }
    }

    pub fn build_query(&self) -> Value {
        use serde_json::json;
        if self.filters.is_empty() {
            json!({})
        } else if self.filters.len() == 1 {
            self.filters[0].to_query()
        } else {
            let conditions: Vec<Value> = self.filters.iter().map(|f| f.to_query()).collect();
            json!({ "$and": conditions })
        }
    }

    pub fn insert_char(&mut self, c: char) {
        match self.focus {
            FilterFocus::Field => {
                // Convert character position to byte position for UTF-8 safety
                let byte_pos = self
                    .field_input
                    .char_indices()
                    .nth(self.field_cursor)
                    .map(|(i, _)| i)
                    .unwrap_or(self.field_input.len());
                self.field_input.insert(byte_pos, c);
                self.field_cursor += 1;
                self.update_suggestions();
            }
            FilterFocus::Value => {
                let byte_pos = self
                    .value_input
                    .char_indices()
                    .nth(self.value_cursor)
                    .map(|(i, _)| i)
                    .unwrap_or(self.value_input.len());
                self.value_input.insert(byte_pos, c);
                self.value_cursor += 1;
            }
            _ => {}
        }
    }

    pub fn backspace(&mut self) {
        match self.focus {
            FilterFocus::Field => {
                if self.field_cursor > 0 {
                    self.field_cursor -= 1;
                    // Convert character position to byte position for UTF-8 safety
                    let byte_pos = self
                        .field_input
                        .char_indices()
                        .nth(self.field_cursor)
                        .map(|(i, _)| i)
                        .unwrap_or(0);
                    self.field_input.remove(byte_pos);
                    self.update_suggestions();
                }
            }
            FilterFocus::Value => {
                if self.value_cursor > 0 {
                    self.value_cursor -= 1;
                    let byte_pos = self
                        .value_input
                        .char_indices()
                        .nth(self.value_cursor)
                        .map(|(i, _)| i)
                        .unwrap_or(0);
                    self.value_input.remove(byte_pos);
                }
            }
            _ => {}
        }
    }

    pub fn cursor_left(&mut self) {
        match self.focus {
            FilterFocus::Field => {
                if self.field_cursor > 0 {
                    self.field_cursor -= 1;
                }
            }
            FilterFocus::Value => {
                if self.value_cursor > 0 {
                    self.value_cursor -= 1;
                }
            }
            _ => {}
        }
    }

    pub fn cursor_right(&mut self) {
        match self.focus {
            FilterFocus::Field => {
                // Use character count, not byte length
                if self.field_cursor < self.field_input.chars().count() {
                    self.field_cursor += 1;
                }
            }
            FilterFocus::Value => {
                if self.value_cursor < self.value_input.chars().count() {
                    self.value_cursor += 1;
                }
            }
            _ => {}
        }
    }

    pub fn cursor_home(&mut self) {
        match self.focus {
            FilterFocus::Field => self.field_cursor = 0,
            FilterFocus::Value => self.value_cursor = 0,
            _ => {}
        }
    }

    pub fn cursor_end(&mut self) {
        match self.focus {
            // Use character count, not byte length
            FilterFocus::Field => self.field_cursor = self.field_input.chars().count(),
            FilterFocus::Value => self.value_cursor = self.value_input.chars().count(),
            _ => {}
        }
    }

    /// Set available fields from collection schema
    pub fn set_fields(&mut self, fields: Vec<String>) {
        self.all_fields = fields;
        // Show suggestions immediately when fields are loaded
        self.update_suggestions();
        self.show_suggestions = true;
    }

    /// Update filtered suggestions based on current input
    pub fn update_suggestions(&mut self) {
        let input = self.field_input.to_lowercase();
        if input.is_empty() {
            // Show all fields when input is empty
            self.filtered_suggestions = self.all_fields.clone();
        } else {
            // Filter fields that contain the input
            self.filtered_suggestions = self
                .all_fields
                .iter()
                .filter(|f| f.to_lowercase().contains(&input))
                .cloned()
                .collect();
        }
        // Reset selection if out of bounds
        if self.suggestion_idx >= self.filtered_suggestions.len() {
            self.suggestion_idx = 0;
        }
        // Show suggestions when there are matches
        self.show_suggestions = !self.filtered_suggestions.is_empty();
    }

    /// Select next suggestion (down arrow)
    pub fn suggestion_down(&mut self) {
        if !self.filtered_suggestions.is_empty() {
            self.suggestion_idx = (self.suggestion_idx + 1) % self.filtered_suggestions.len();
        }
    }

    /// Select previous suggestion (up arrow)
    pub fn suggestion_up(&mut self) {
        if !self.filtered_suggestions.is_empty() {
            if self.suggestion_idx == 0 {
                self.suggestion_idx = self.filtered_suggestions.len() - 1;
            } else {
                self.suggestion_idx -= 1;
            }
        }
    }

    /// Apply selected suggestion to field input
    pub fn apply_suggestion(&mut self) {
        if let Some(suggestion) = self.filtered_suggestions.get(self.suggestion_idx) {
            self.field_input = suggestion.clone();
            // Use character count, not byte length for UTF-8 safety
            self.field_cursor = self.field_input.chars().count();
            self.show_suggestions = false;
        }
    }

    /// Toggle suggestions visibility
    pub fn toggle_suggestions(&mut self) {
        if self.focus == FilterFocus::Field {
            self.show_suggestions = !self.show_suggestions;
            if self.show_suggestions {
                self.update_suggestions();
            }
        }
    }
}

/// Editor mode - insert or edit
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EditorMode {
    #[default]
    Insert,
    Edit,
}

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

/// Index management state
#[derive(Debug, Clone, Default)]
pub struct IndexState {
    pub collection: String,
    pub indexes: Vec<String>,
    pub selected_index: usize,
    pub is_creating: bool,
    pub form_field: usize, // 0 = field name, 1 = unique checkbox
    pub field_input: String,
    pub unique: bool,
    pub error: Option<String>,
    pub message: Option<String>,
}

impl IndexState {
    pub fn new(collection: String, indexes: Vec<String>) -> Self {
        Self {
            collection,
            indexes,
            selected_index: 0,
            is_creating: false,
            form_field: 0,
            field_input: String::new(),
            unique: false,
            error: None,
            message: None,
        }
    }

    pub fn select_up(&mut self) {
        if self.selected_index > 0 {
            self.selected_index -= 1;
        }
    }

    pub fn select_down(&mut self) {
        if self.selected_index + 1 < self.indexes.len() {
            self.selected_index += 1;
        }
    }

    pub fn start_create(&mut self) {
        self.is_creating = true;
        self.form_field = 0;
        self.field_input.clear();
        self.unique = false;
        self.error = None;
    }

    pub fn cancel_create(&mut self) {
        self.is_creating = false;
        self.field_input.clear();
        self.unique = false;
        self.error = None;
    }

    pub fn next_form_field(&mut self) {
        self.form_field = (self.form_field + 1) % 2;
    }

    pub fn toggle_unique(&mut self) {
        self.unique = !self.unique;
    }

    pub fn insert_char(&mut self, c: char) {
        self.field_input.push(c);
        self.error = None;
    }

    pub fn backspace(&mut self) {
        self.field_input.pop();
        self.error = None;
    }
}

/// Query builder state
#[derive(Debug, Clone)]
pub struct QueryState {
    pub collection: String,
    pub lines: Vec<String>,
    pub cursor_line: usize,
    pub cursor_col: usize,
    pub error: Option<String>,
    pub results: Option<Vec<Value>>,
    /// Show template selector
    pub show_templates: bool,
    /// Selected template index
    pub template_index: usize,
    /// Selected result index for navigation
    pub result_index: usize,
    /// Total result count (may be > results.len() if truncated)
    pub total_count: usize,
}

/// Query templates
pub const QUERY_TEMPLATES: &[(&str, &str)] = &[
    ("Osszes", "{}"),
    ("ID alapjan", "{\"_id\": \"\"}"),
    ("Egyenloseg", "{\"field\": \"value\"}"),
    ("Nagyobb mint", "{\"field\": {\"$gt\": 0}}"),
    ("Kisebb mint", "{\"field\": {\"$lt\": 100}}"),
    ("Tartalmazza", "{\"field\": {\"$in\": [\"a\", \"b\"]}}"),
    ("Letezik", "{\"field\": {\"$exists\": true}}"),
    ("Regex", "{\"field\": {\"$regex\": \"pattern\"}}"),
    ("AND", "{\"$and\": [{\"a\": 1}, {\"b\": 2}]}"),
    ("OR", "{\"$or\": [{\"a\": 1}, {\"b\": 2}]}"),
];

impl Default for QueryState {
    fn default() -> Self {
        Self {
            collection: String::new(),
            lines: vec!["{}".to_string()],
            cursor_line: 0,
            cursor_col: 1,
            error: None,
            results: None,
            show_templates: false,
            template_index: 0,
            result_index: 0,
            total_count: 0,
        }
    }
}

impl QueryState {
    pub fn new(collection: String) -> Self {
        Self {
            collection,
            lines: vec!["{".to_string(), "  ".to_string(), "}".to_string()],
            cursor_line: 1,
            cursor_col: 2,
            error: None,
            results: None,
            show_templates: false,
            template_index: 0,
            result_index: 0,
            total_count: 0,
        }
    }

    /// Check if we have results to navigate
    pub fn has_results(&self) -> bool {
        self.results.as_ref().map(|r| !r.is_empty()).unwrap_or(false)
    }

    /// Navigate to next result
    pub fn result_down(&mut self) {
        if let Some(ref results) = self.results {
            if self.result_index + 1 < results.len() {
                self.result_index += 1;
            }
        }
    }

    /// Navigate to previous result
    pub fn result_up(&mut self) {
        if self.result_index > 0 {
            self.result_index -= 1;
        }
    }

    /// Get current query string
    pub fn get_query_string(&self) -> String {
        self.lines.join("\n")
    }

    /// Clear results (when user edits query)
    pub fn clear_results(&mut self) {
        self.results = None;
        self.result_index = 0;
        self.total_count = 0;
    }

    /// Toggle template selector
    pub fn toggle_templates(&mut self) {
        self.show_templates = !self.show_templates;
        self.template_index = 0;
    }

    /// Select next template
    pub fn template_down(&mut self) {
        if self.template_index + 1 < QUERY_TEMPLATES.len() {
            self.template_index += 1;
        }
    }

    /// Select previous template
    pub fn template_up(&mut self) {
        if self.template_index > 0 {
            self.template_index -= 1;
        }
    }

    /// Apply selected template
    pub fn apply_template(&mut self) {
        if let Some((_, template)) = QUERY_TEMPLATES.get(self.template_index) {
            self.lines = vec![template.to_string()];
            self.cursor_line = 0;
            self.cursor_col = template.chars().count();
            self.show_templates = false;
            self.error = None;
        }
    }

    pub fn insert_char(&mut self, c: char) {
        // Clear results when editing
        self.clear_results();

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

    pub fn backspace(&mut self) {
        // Clear results when editing
        self.clear_results();

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
            let current_line = self.lines.remove(self.cursor_line);
            self.cursor_line -= 1;
            if let Some(prev_line) = self.lines.get_mut(self.cursor_line) {
                self.cursor_col = prev_line.chars().count();
                prev_line.push_str(&current_line);
            }
        }
        self.error = None;
    }

    pub fn insert_newline(&mut self) {
        // Clear results when editing
        self.clear_results();

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

    pub fn insert_tab(&mut self) {
        self.insert_char(' ');
        self.insert_char(' ');
    }

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

    pub fn get_query_text(&self) -> String {
        self.lines.join("\n")
    }

    pub fn parse_query(&self) -> Result<Value, String> {
        let text = self.get_query_text();
        serde_json::from_str(&text).map_err(|e| e.to_string())
    }
}

/// Export state
#[derive(Debug, Clone)]
pub struct ExportState {
    pub collection: String,
    pub doc_count: usize,
    pub format: ExportFormat,
    pub file_path: String,
    pub editing_path: bool,
    pub error: Option<String>,
    pub message: Option<String>,
}

impl Default for ExportState {
    fn default() -> Self {
        Self {
            collection: String::new(),
            doc_count: 0,
            format: ExportFormat::Json,
            file_path: String::new(),
            editing_path: true,
            error: None,
            message: None,
        }
    }
}

impl ExportState {
    pub fn new(collection: String, doc_count: usize) -> Self {
        let file_path = format!("{}.json", collection);
        Self {
            collection,
            doc_count,
            format: ExportFormat::Json,
            file_path,
            editing_path: true,
            error: None,
            message: None,
        }
    }

    pub fn toggle_format(&mut self) {
        self.format = match self.format {
            ExportFormat::Json => ExportFormat::Csv,
            ExportFormat::Csv => ExportFormat::Json,
        };
        // Update extension
        self.update_extension();
    }

    pub fn set_format(&mut self, format: ExportFormat) {
        self.format = format;
        self.update_extension();
    }

    fn update_extension(&mut self) {
        // Replace extension in path
        if let Some(dot_pos) = self.file_path.rfind('.') {
            self.file_path.truncate(dot_pos);
        }
        self.file_path.push('.');
        self.file_path.push_str(self.format.extension());
    }

    pub fn insert_char(&mut self, c: char) {
        self.file_path.push(c);
        self.error = None;
    }

    pub fn backspace(&mut self) {
        self.file_path.pop();
        self.error = None;
    }
}

/// Database open/create mode
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DatabaseMode {
    #[default]
    Open,
    Create,
}

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

// ============================================================
// IronRhai Script Editor State
// ============================================================

/// Script mode - browse, edit, new, history, inline
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ScriptMode {
    #[default]
    Browse,
    Edit,
    New,
    History,
    Inline,
}

/// Pending confirmation action for script modal
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScriptConfirmAction {
    /// Discard unsaved changes and go back
    DiscardChanges,
    /// Delete the selected script
    DeleteScript,
}

/// Script editor focus area
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ScriptFocus {
    #[default]
    List,
    Name,
    Description,
    Tags,
    Editor,
    Params,
    History,
}

/// Script info for browse list (without code)
#[derive(Debug, Clone)]
pub struct ScriptInfo {
    pub name: String,
    pub description: Option<String>,
    pub version: u32,
    pub tags: Vec<String>,
    pub execution_count: u64,
    pub last_run_at: Option<String>,
}

/// Script version for history
#[derive(Debug, Clone)]
pub struct ScriptVersion {
    pub version: u32,
    pub code: String,
    pub description: Option<String>,
    pub created_at: String,
}

/// Script execution result
#[derive(Debug, Clone)]
pub struct ScriptResult {
    pub success: bool,
    pub result: Value,
    pub logs: Vec<String>,
    pub error: Option<String>,
    pub execution_time_ms: u64,
}

/// IronRhai script editor state
#[derive(Debug, Clone)]
pub struct ScriptState {
    // Editor
    pub lines: Vec<String>,
    pub cursor_line: usize,
    pub cursor_col: usize,
    pub scroll_offset: usize,

    // Metadata
    pub name: String,
    pub name_cursor: usize,
    pub description: String,
    pub desc_cursor: usize,
    pub tags: Vec<String>,
    pub selected_tag: usize,
    pub tag_input: String,
    pub tag_input_active: bool,
    pub version: u32,

    // Execution
    pub params_input: String,
    pub params_cursor: usize,
    pub result: Option<ScriptResult>,

    // UI state
    pub focus: ScriptFocus,
    pub mode: ScriptMode,
    pub error: Option<String>,
    pub message: Option<String>,
    pub dirty: bool,
    pub loading: bool,
    pub confirm_action: Option<ScriptConfirmAction>,

    // Browse mode
    pub scripts: Vec<ScriptInfo>,
    pub selected_script: usize,

    // History mode
    pub versions: Vec<ScriptVersion>,
    pub selected_version: usize,
}

impl Default for ScriptState {
    fn default() -> Self {
        Self {
            lines: vec!["// IronRhai script".to_string(), "".to_string()],
            cursor_line: 1,
            cursor_col: 0,
            scroll_offset: 0,
            name: String::new(),
            name_cursor: 0,
            description: String::new(),
            desc_cursor: 0,
            tags: Vec::new(),
            selected_tag: 0,
            tag_input: String::new(),
            tag_input_active: false,
            version: 0,
            params_input: "{}".to_string(),
            params_cursor: 2,
            result: None,
            focus: ScriptFocus::List,
            mode: ScriptMode::Browse,
            error: None,
            message: None,
            dirty: false,
            loading: false,
            confirm_action: None,
            scripts: Vec::new(),
            selected_script: 0,
            versions: Vec::new(),
            selected_version: 0,
        }
    }
}

impl ScriptState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Reset to browse mode
    pub fn reset_to_browse(&mut self) {
        self.mode = ScriptMode::Browse;
        self.focus = ScriptFocus::List;
        self.error = None;
        self.message = None;
        self.dirty = false;
        self.confirm_action = None;
    }

    /// Start new script
    pub fn start_new(&mut self) {
        self.mode = ScriptMode::New;
        self.focus = ScriptFocus::Name;
        self.lines = vec!["// IronRhai script".to_string(), "".to_string()];
        self.cursor_line = 1;
        self.cursor_col = 0;
        self.scroll_offset = 0;
        self.name.clear();
        self.name_cursor = 0;
        self.description.clear();
        self.desc_cursor = 0;
        self.tags.clear();
        self.tag_input.clear();
        self.tag_input_active = false;
        self.version = 0;
        self.params_input = "{}".to_string();
        self.params_cursor = 2;
        self.result = None;
        self.error = None;
        self.message = None;
        self.dirty = false;
    }

    /// Load script for editing
    pub fn load_script(&mut self, name: String, code: String, desc: Option<String>, tags: Vec<String>, version: u32) {
        self.mode = ScriptMode::Edit;
        self.focus = ScriptFocus::Editor;
        self.lines = code.lines().map(String::from).collect();
        if self.lines.is_empty() {
            self.lines.push(String::new());
        }
        self.cursor_line = 0;
        self.cursor_col = 0;
        self.scroll_offset = 0;
        self.name = name;
        self.name_cursor = self.name.chars().count();
        self.description = desc.unwrap_or_default();
        self.desc_cursor = self.description.chars().count();
        self.tags = tags;
        self.selected_tag = 0;
        self.tag_input.clear();
        self.tag_input_active = false;
        self.version = version;
        self.result = None;
        self.error = None;
        self.message = None;
        self.dirty = false;
    }

    /// Switch to history mode
    pub fn enter_history(&mut self) {
        self.mode = ScriptMode::History;
        self.focus = ScriptFocus::History;
        self.selected_version = 0;
    }

    /// Switch to inline mode (ad-hoc execution)
    pub fn enter_inline(&mut self) {
        self.mode = ScriptMode::Inline;
        self.focus = ScriptFocus::Editor;
        self.lines = vec!["// Ad-hoc script".to_string(), "".to_string()];
        self.cursor_line = 1;
        self.cursor_col = 0;
        self.scroll_offset = 0;
        self.name = "[inline]".to_string();
        self.description.clear();
        self.tags.clear();
        self.version = 0;
        self.result = None;
        self.error = None;
        self.dirty = false;
    }

    // === Editor operations ===

    pub fn insert_char(&mut self, c: char) {
        match self.focus {
            ScriptFocus::Name => {
                let byte_pos = self.name.char_indices().nth(self.name_cursor).map(|(i, _)| i).unwrap_or(self.name.len());
                self.name.insert(byte_pos, c);
                self.name_cursor += 1;
                self.dirty = true;
            }
            ScriptFocus::Description => {
                let byte_pos = self.description.char_indices().nth(self.desc_cursor).map(|(i, _)| i).unwrap_or(self.description.len());
                self.description.insert(byte_pos, c);
                self.desc_cursor += 1;
                self.dirty = true;
            }
            ScriptFocus::Tags if self.tag_input_active => {
                self.tag_input.push(c);
            }
            ScriptFocus::Editor => {
                if let Some(line) = self.lines.get_mut(self.cursor_line) {
                    let char_count = line.chars().count();
                    if self.cursor_col <= char_count {
                        let byte_pos = line.char_indices().nth(self.cursor_col).map(|(i, _)| i).unwrap_or(line.len());
                        line.insert(byte_pos, c);
                        self.cursor_col += 1;
                        self.dirty = true;
                    }
                }
            }
            ScriptFocus::Params => {
                let byte_pos = self.params_input.char_indices().nth(self.params_cursor).map(|(i, _)| i).unwrap_or(self.params_input.len());
                self.params_input.insert(byte_pos, c);
                self.params_cursor += 1;
            }
            _ => {}
        }
        self.error = None;
    }

    pub fn backspace(&mut self) {
        match self.focus {
            ScriptFocus::Name if self.name_cursor > 0 => {
                let byte_pos = self.name.char_indices().nth(self.name_cursor - 1).map(|(i, _)| i).unwrap_or(0);
                self.name.remove(byte_pos);
                self.name_cursor -= 1;
                self.dirty = true;
            }
            ScriptFocus::Description if self.desc_cursor > 0 => {
                let byte_pos = self.description.char_indices().nth(self.desc_cursor - 1).map(|(i, _)| i).unwrap_or(0);
                self.description.remove(byte_pos);
                self.desc_cursor -= 1;
                self.dirty = true;
            }
            ScriptFocus::Tags if self.tag_input_active => {
                self.tag_input.pop();
            }
            ScriptFocus::Editor => {
                if self.cursor_col > 0 {
                    if let Some(line) = self.lines.get_mut(self.cursor_line) {
                        let byte_pos = line.char_indices().nth(self.cursor_col - 1).map(|(i, _)| i).unwrap_or(0);
                        line.remove(byte_pos);
                        self.cursor_col -= 1;
                        self.dirty = true;
                    }
                } else if self.cursor_line > 0 {
                    let current_line = self.lines.remove(self.cursor_line);
                    self.cursor_line -= 1;
                    if let Some(prev_line) = self.lines.get_mut(self.cursor_line) {
                        self.cursor_col = prev_line.chars().count();
                        prev_line.push_str(&current_line);
                    }
                    self.dirty = true;
                }
            }
            ScriptFocus::Params if self.params_cursor > 0 => {
                let byte_pos = self.params_input.char_indices().nth(self.params_cursor - 1).map(|(i, _)| i).unwrap_or(0);
                self.params_input.remove(byte_pos);
                self.params_cursor -= 1;
            }
            _ => {}
        }
        self.error = None;
    }

    pub fn insert_newline(&mut self) {
        if self.focus == ScriptFocus::Editor {
            if let Some(line) = self.lines.get_mut(self.cursor_line) {
                let byte_pos = line.char_indices().nth(self.cursor_col).map(|(i, _)| i).unwrap_or(line.len());
                let rest = line.split_off(byte_pos);
                self.cursor_line += 1;
                self.lines.insert(self.cursor_line, rest);
                self.cursor_col = 0;
                self.dirty = true;
            }
        }
        self.error = None;
    }

    pub fn insert_tab(&mut self) {
        if self.focus == ScriptFocus::Editor {
            self.insert_char(' ');
            self.insert_char(' ');
        }
    }

    // === Cursor navigation ===

    pub fn cursor_left(&mut self) {
        match self.focus {
            ScriptFocus::Name if self.name_cursor > 0 => self.name_cursor -= 1,
            ScriptFocus::Description if self.desc_cursor > 0 => self.desc_cursor -= 1,
            ScriptFocus::Editor => {
                if self.cursor_col > 0 {
                    self.cursor_col -= 1;
                } else if self.cursor_line > 0 {
                    self.cursor_line -= 1;
                    self.cursor_col = self.lines.get(self.cursor_line).map(|l| l.chars().count()).unwrap_or(0);
                }
            }
            ScriptFocus::Params if self.params_cursor > 0 => self.params_cursor -= 1,
            _ => {}
        }
    }

    pub fn cursor_right(&mut self) {
        match self.focus {
            ScriptFocus::Name if self.name_cursor < self.name.chars().count() => self.name_cursor += 1,
            ScriptFocus::Description if self.desc_cursor < self.description.chars().count() => self.desc_cursor += 1,
            ScriptFocus::Editor => {
                let char_count = self.lines.get(self.cursor_line).map(|l| l.chars().count()).unwrap_or(0);
                if self.cursor_col < char_count {
                    self.cursor_col += 1;
                } else if self.cursor_line + 1 < self.lines.len() {
                    self.cursor_line += 1;
                    self.cursor_col = 0;
                }
            }
            ScriptFocus::Params if self.params_cursor < self.params_input.chars().count() => self.params_cursor += 1,
            _ => {}
        }
    }

    pub fn cursor_up(&mut self) {
        match self.focus {
            ScriptFocus::Editor if self.cursor_line > 0 => {
                self.cursor_line -= 1;
                let char_count = self.lines.get(self.cursor_line).map(|l| l.chars().count()).unwrap_or(0);
                self.cursor_col = self.cursor_col.min(char_count);
            }
            ScriptFocus::List if self.selected_script > 0 => self.selected_script -= 1,
            ScriptFocus::History if self.selected_version > 0 => self.selected_version -= 1,
            _ => {}
        }
    }

    pub fn cursor_down(&mut self) {
        match self.focus {
            ScriptFocus::Editor if self.cursor_line + 1 < self.lines.len() => {
                self.cursor_line += 1;
                let char_count = self.lines.get(self.cursor_line).map(|l| l.chars().count()).unwrap_or(0);
                self.cursor_col = self.cursor_col.min(char_count);
            }
            ScriptFocus::List if self.selected_script + 1 < self.scripts.len() => self.selected_script += 1,
            ScriptFocus::History if self.selected_version + 1 < self.versions.len() => self.selected_version += 1,
            _ => {}
        }
    }

    // === Focus navigation ===

    pub fn next_focus(&mut self) {
        self.focus = match self.focus {
            ScriptFocus::Name => ScriptFocus::Description,
            ScriptFocus::Description => ScriptFocus::Tags,
            ScriptFocus::Tags => ScriptFocus::Editor,
            ScriptFocus::Editor => ScriptFocus::Params,
            ScriptFocus::Params => ScriptFocus::Name,
            other => other,
        };
        self.tag_input_active = false;
    }

    pub fn prev_focus(&mut self) {
        self.focus = match self.focus {
            ScriptFocus::Name => ScriptFocus::Params,
            ScriptFocus::Description => ScriptFocus::Name,
            ScriptFocus::Tags => ScriptFocus::Description,
            ScriptFocus::Editor => ScriptFocus::Tags,
            ScriptFocus::Params => ScriptFocus::Editor,
            other => other,
        };
        self.tag_input_active = false;
    }

    pub fn cursor_home(&mut self) {
        match self.focus {
            ScriptFocus::Name => self.name_cursor = 0,
            ScriptFocus::Description => self.desc_cursor = 0,
            ScriptFocus::Editor => self.cursor_col = 0,
            ScriptFocus::Params => self.params_cursor = 0,
            _ => {}
        }
    }

    pub fn cursor_end(&mut self) {
        match self.focus {
            ScriptFocus::Name => self.name_cursor = self.name.chars().count(),
            ScriptFocus::Description => self.desc_cursor = self.description.chars().count(),
            ScriptFocus::Editor => {
                self.cursor_col = self.lines.get(self.cursor_line).map(|l| l.chars().count()).unwrap_or(0);
            }
            ScriptFocus::Params => self.params_cursor = self.params_input.chars().count(),
            _ => {}
        }
    }

    pub fn delete_char(&mut self) {
        if self.focus == ScriptFocus::Editor {
            if let Some(line) = self.lines.get_mut(self.cursor_line) {
                let char_count = line.chars().count();
                if self.cursor_col < char_count {
                    let byte_pos = line.char_indices().nth(self.cursor_col).map(|(i, _)| i).unwrap_or(line.len());
                    let next_byte_pos = line.char_indices().nth(self.cursor_col + 1).map(|(i, _)| i).unwrap_or(line.len());
                    line.replace_range(byte_pos..next_byte_pos, "");
                    self.dirty = true;
                } else if self.cursor_line + 1 < self.lines.len() {
                    // Merge next line
                    let next_line = self.lines.remove(self.cursor_line + 1);
                    if let Some(current_line) = self.lines.get_mut(self.cursor_line) {
                        current_line.push_str(&next_line);
                    }
                    self.dirty = true;
                }
            }
        }
        self.error = None;
    }

    // === Tag operations ===

    pub fn add_tag(&mut self) {
        if !self.tag_input.is_empty() {
            let tag = self.tag_input.trim().to_string();
            if !self.tags.contains(&tag) {
                self.tags.push(tag);
                self.dirty = true;
            }
            self.tag_input.clear();
            self.tag_input_active = false;
        }
    }

    pub fn remove_selected_tag(&mut self) {
        if !self.tags.is_empty() && self.selected_tag < self.tags.len() {
            self.tags.remove(self.selected_tag);
            if self.selected_tag > 0 {
                self.selected_tag -= 1;
            }
            self.dirty = true;
        }
    }

    pub fn select_prev_tag(&mut self) {
        if self.selected_tag > 0 {
            self.selected_tag -= 1;
        }
    }

    pub fn select_next_tag(&mut self) {
        if self.selected_tag + 1 < self.tags.len() {
            self.selected_tag += 1;
        }
    }

    pub fn toggle_tag_input(&mut self) {
        self.tag_input_active = !self.tag_input_active;
        if self.tag_input_active {
            self.tag_input.clear();
        }
    }

    // === Utility ===

    pub fn get_code(&self) -> String {
        self.lines.join("\n")
    }

    pub fn get_selected_script(&self) -> Option<&ScriptInfo> {
        self.scripts.get(self.selected_script)
    }

    pub fn get_selected_version(&self) -> Option<&ScriptVersion> {
        self.versions.get(self.selected_version)
    }
}

/// Main application state
pub struct App {
    // Quit flag
    pub should_quit: bool,

    // Focus management
    pub active_pane: Pane,
    pub modal: Option<Modal>,
    pub help_scroll: usize,

    // Theme
    pub theme_name: ThemeName,
    pub theme: Theme,

    // Database
    pub db: Option<DbWrapper>,
    pub db_path: Option<PathBuf>,

    // Collections pane
    pub collections: Vec<CollectionInfo>,
    pub selected_collection: usize,
    pub collections_scroll: usize,

    // Documents pane
    pub documents: Vec<Value>,
    pub selected_document: usize,
    pub doc_scroll_offset: usize,
    pub page_size: usize,
    pub total_docs: usize,
    pub active_filter: Option<Value>, // Active filter query for pagination
    pub active_sort: Option<Value>,   // Active sort for pagination

    // Detail pane
    pub detail_scroll: usize,

    // Search state
    pub search: SearchState,

    // Confirm dialog state
    pub confirm: ConfirmState,

    // Insert dialog state
    pub insert: InsertState,

    // Index management state
    pub index_state: IndexState,

    // Query builder state
    pub query_state: QueryState,

    // Visual filter state
    pub filter_state: FilterState,

    // Export state
    pub export_state: ExportState,

    // New collection state
    pub new_collection_state: NewCollectionState,

    // IronRhai script editor state
    pub script_state: ScriptState,

    // Server info modal state
    pub server_info_state: crate::modals::server_info::ServerInfoState,
    pub server_info_scroll: usize,

    // Update modal state
    pub update_state: crate::modals::update::UpdateState,

    // Database open/create modal state
    pub database_state: DatabaseState,

    // API Key modal state
    pub api_key_state: crate::modals::api_key::ApiKeyState,

    // Config
    pub config: Config,

    // Status
    pub status_message: Option<String>,
    pub error_message: Option<String>,
    pub error_scroll: usize,
}

impl App {
    pub fn new(config: Config) -> Self {
        let theme_name = config.theme;
        let theme = Theme::from_name(theme_name);

        Self {
            should_quit: false,
            active_pane: Pane::Collections,
            modal: None,
            help_scroll: 0,
            theme_name,
            theme,
            db: None,
            db_path: None,
            collections: Vec::new(),
            selected_collection: 0,
            collections_scroll: 0,
            documents: Vec::new(),
            selected_document: 0,
            doc_scroll_offset: 0,
            page_size: 20,
            total_docs: 0,
            active_filter: None,
            active_sort: None,
            detail_scroll: 0,
            search: SearchState::new(),
            confirm: ConfirmState::default(),
            insert: InsertState::default(),
            index_state: IndexState::default(),
            query_state: QueryState::default(),
            filter_state: FilterState::default(),
            export_state: ExportState::default(),
            new_collection_state: NewCollectionState::default(),
            script_state: ScriptState::default(),
            server_info_state: crate::modals::server_info::ServerInfoState::new(),
            server_info_scroll: 0,
            update_state: crate::modals::update::UpdateState::default(),
            database_state: DatabaseState::default(),
            api_key_state: crate::modals::api_key::ApiKeyState::new(),
            config,
            status_message: None,
            error_message: None,
            error_scroll: 0,
        }
    }

    /// Update page_size based on terminal height
    /// Returns true if page_size changed (requires document refresh)
    pub fn update_page_size(&mut self, terminal_height: u16) -> bool {
        // Calculate available height for documents:
        // terminal_height - header(1) - command_bar(1) - borders(2) = inner height
        let new_page_size = terminal_height
            .saturating_sub(TERMINAL_OVERHEAD)
            .max(MIN_PAGE_SIZE) as usize;

        if new_page_size != self.page_size {
            self.page_size = new_page_size;
            true
        } else {
            false
        }
    }

    // NOTE: Database connection is now handled asynchronously in main.rs
    // The old synchronous open_database and refresh methods have been replaced
    // with async versions: refresh_collections_async, refresh_documents_async

    /// Get collections for rendering
    pub fn get_collections(&self) -> &[CollectionInfo] {
        &self.collections
    }

    /// Get documents for rendering
    pub fn get_documents(&self) -> &[Value] {
        &self.documents
    }

    /// Get selected document
    pub fn get_selected_document(&self) -> Option<&Value> {
        self.documents.get(
            self.selected_document
                .saturating_sub(self.doc_scroll_offset),
        )
    }

    /// Get line count of selected document (for scroll bounds)
    pub fn get_selected_document_lines(&self) -> usize {
        self.get_selected_document()
            .and_then(|doc| serde_json::to_string_pretty(doc).ok())
            .map(|s| s.lines().count())
            .unwrap_or(0)
    }

    /// Get cached indexes for current collection (for sync render)
    pub fn get_current_indexes(&self) -> &[String] {
        &self.index_state.indexes
    }

    /// Check if a collection is selected
    pub fn has_collection(&self) -> bool {
        !self.collections.is_empty()
    }

    /// Get current collection name
    pub fn current_collection_name(&self) -> Option<&str> {
        self.collections
            .get(self.selected_collection)
            .map(|c| c.name.as_str())
    }

    /// Get pagination info (current_page, total_pages)
    pub fn get_pagination_info(&self) -> (usize, usize) {
        if self.total_docs == 0 {
            return (0, 0);
        }
        let current_page = (self.doc_scroll_offset / self.page_size) + 1;
        let total_pages = (self.total_docs + self.page_size - 1) / self.page_size;
        (current_page, total_pages)
    }

    // === Navigation ===

    /// Switch to next pane
    pub fn next_pane(&mut self) {
        self.active_pane = self.active_pane.next();
    }

    /// Switch to previous pane
    pub fn prev_pane(&mut self) {
        self.active_pane = self.active_pane.prev();
    }

    // Sync select_up/down/page_up/page_down/go_to_start/go_to_end törölve
    // Használd az async verziókat: select_up_async, select_down_async, stb.

    // === Modals ===

    /// Open search modal - mode depends on active pane
    pub fn open_search(&mut self) {
        self.modal = Some(Modal::Search);
        self.search.input_active = true;
        self.search.query_input.clear();
        self.search.cursor_pos = 0;
        self.search.results.clear();
        self.search.selected_result = 0;
        self.search.doc_matches.clear();
        self.search.current_match = 0;

        // Set mode based on active pane
        self.search.mode =
            if self.active_pane == Pane::Detail && self.get_selected_document().is_some() {
                SearchMode::Document
            } else {
                SearchMode::Collections
            };
    }

    // execute_search() és goto_search_result() törölve
    // Használd az async verziókat: execute_search_async, goto_search_result_async

    /// Search result navigation
    pub fn search_select_up(&mut self) {
        if self.search.selected_result > 0 {
            self.search.selected_result -= 1;
            if self.search.selected_result < self.search.result_offset {
                self.search.result_offset = self.search.selected_result;
            }
        }
    }

    pub fn search_select_down(&mut self) {
        if self.search.selected_result + 1 < self.search.results.len() {
            self.search.selected_result += 1;
            // Scroll down if needed (assume ~10 visible)
            if self.search.selected_result >= self.search.result_offset + 10 {
                self.search.result_offset = self.search.selected_result.saturating_sub(9);
            }
        }
    }

    /// Open actions modal
    pub fn open_actions(&mut self) {
        self.modal = Some(Modal::Actions);
    }

    /// Open help modal
    pub fn open_help(&mut self) {
        self.modal = Some(Modal::Help);
    }

    /// Open server info modal
    pub fn open_server_info(&mut self) {
        self.server_info_scroll = 0;
        self.server_info_state = crate::modals::server_info::ServerInfoState::new();
        self.modal = Some(Modal::ServerInfo);
    }

    /// Update server info state with data from MCP server
    pub fn update_server_info(&mut self, db_stats: serde_json::Value, tools: Vec<serde_json::Value>, prompts: Vec<serde_json::Value>) {
        self.server_info_state.update(db_stats, tools, prompts);
    }

    /// Set server info error
    pub fn set_server_info_error(&mut self, error: String) {
        self.server_info_state.set_error(error);
    }

    /// Open update check modal
    pub fn open_update(&mut self, current_version: String) {
        self.update_state = crate::modals::update::UpdateState::new(current_version);
        self.modal = Some(Modal::Update);
    }

    /// Update the update state with GitHub data
    pub fn update_update_state(&mut self, latest_version: String, download_url: String, release_notes: Option<String>) {
        self.update_state.update_from_github(latest_version, download_url, release_notes);
    }

    /// Set update check error
    pub fn set_update_error(&mut self, error: String) {
        self.update_state.set_error(error);
    }

    /// Close current modal
    pub fn close_modal(&mut self) {
        self.modal = None;
    }

    // === Delete Document ===

    /// Open delete confirmation for selected document
    pub fn open_delete_confirm(&mut self) {
        // Get current document and collection
        let Some(doc) = self.get_selected_document().cloned() else {
            self.set_error("Nincs kivalasztott dokumentum");
            return;
        };

        let Some(coll_name) = self.current_collection_name().map(String::from) else {
            self.set_error("Nincs kivalasztott kollekció");
            return;
        };

        let doc_id = doc.get("_id").cloned().unwrap_or(Value::Null);
        let preview = serde_json::to_string_pretty(&doc).unwrap_or_else(|_| "{}".to_string());

        self.confirm = ConfirmState::delete_document(coll_name, doc_id, &preview);
        self.modal = Some(Modal::Confirm);
    }

    /// Open delete confirmation for selected collection
    pub fn open_delete_collection_confirm(&mut self) {
        let Some(coll) = self.collections.get(self.selected_collection).cloned() else {
            self.set_error("Nincs kiválasztott kollekció");
            return;
        };

        let doc_count = coll.doc_count.unwrap_or(0);
        self.confirm = ConfirmState::delete_collection(coll.name, doc_count);
        self.modal = Some(Modal::Confirm);
    }

    /// Toggle confirm dialog selection
    pub fn confirm_toggle(&mut self) {
        self.confirm.toggle_selection();
    }

    // execute_confirm_action, do_delete_document, execute_insert törölve
    // Használd: execute_confirm_action_async, execute_insert_async

    // === Insert Document ===

    /// Open insert document modal
    pub fn open_insert(&mut self) {
        let Some(coll_name) = self.current_collection_name().map(String::from) else {
            self.set_error("Nincs kiválasztott kollekció");
            return;
        };

        // Use template from existing document or default
        let template = if let Some(doc) = self.documents.first() {
            Self::create_empty_template(doc)
        } else {
            None
        };

        self.insert = InsertState::with_template(coll_name, template);
        self.modal = Some(Modal::Insert);
    }

    /// Create empty template from existing document (keep structure, clear values)
    fn create_empty_template(doc: &Value) -> Option<Value> {
        fn clear_values(val: &Value) -> Value {
            match val {
                Value::Object(map) => {
                    let mut new_map = serde_json::Map::new();
                    for (k, v) in map {
                        if k == "_id" {
                            // Skip _id - will be auto-generated
                            continue;
                        }
                        new_map.insert(k.clone(), clear_values(v));
                    }
                    Value::Object(new_map)
                }
                Value::Array(_) => Value::Array(vec![]),
                Value::String(_) => Value::String(String::new()),
                Value::Number(_) => Value::Number(0.into()),
                Value::Bool(_) => Value::Bool(false),
                Value::Null => Value::Null,
            }
        }

        if let Value::Object(_) = doc {
            Some(clear_values(doc))
        } else {
            None
        }
    }

    /// Open edit document modal for selected document
    pub fn open_edit(&mut self) {
        let Some(doc) = self.get_selected_document().cloned() else {
            self.set_error("Nincs kiválasztott dokumentum");
            return;
        };

        let Some(coll_name) = self.current_collection_name().map(String::from) else {
            self.set_error("Nincs kiválasztott kollekció");
            return;
        };

        self.insert = InsertState::edit(coll_name, &doc);
        self.modal = Some(Modal::Insert);
    }

    // === Index Management ===

    /// Open index management modal (sync, deferred index loading)
    pub fn open_index_modal(&mut self) {
        let Some(coll_name) = self.current_collection_name().map(String::from) else {
            self.set_error("Nincs kiválasztott kollekció");
            return;
        };

        // Initialize with empty, will be loaded async
        self.index_state = IndexState::new(coll_name, vec![]);
        self.modal = Some(Modal::Index);
    }

    /// Load indexes for current collection (call after opening index modal)
    pub async fn load_indexes_async(&mut self) {
        let collection = self.index_state.collection.clone();
        if collection.is_empty() {
            return;
        }

        let Some(db) = &self.db else {
            return;
        };

        match db.list_indexes(&collection).await {
            Ok(indexes) => {
                self.index_state.indexes = indexes;
            }
            Err(e) => {
                self.index_state.message = Some(format!("Hiba: {}", e));
            }
        }
    }

    // execute_create_index, execute_delete_index törölve
    // Használd az async verziókat: execute_create_index_async, execute_delete_index_async

    // === Query Builder ===

    /// Open query builder modal
    pub fn open_query_modal(&mut self) {
        let Some(coll_name) = self.current_collection_name().map(String::from) else {
            self.set_error("Nincs kiválasztott kollekció");
            return;
        };

        self.query_state = QueryState::new(coll_name);
        self.modal = Some(Modal::Query);
    }

    /// Open visual filter modal (sync - no schema loading)
    pub fn open_filter_modal(&mut self) {
        if self.current_collection_name().is_none() {
            self.set_error("Nincs kiválasztott kollekció");
            return;
        };

        // Preserve existing filters, only reset input fields
        self.filter_state.reset_inputs();
        self.modal = Some(Modal::Filter);
    }

    /// Open visual filter modal with schema loading (async)
    pub async fn open_filter_modal_async(&mut self) {
        let coll_name = match self.current_collection_name() {
            Some(name) => name.to_string(),
            None => {
                self.set_error("Nincs kiválasztott kollekció");
                return;
            }
        };

        // Preserve existing filters, only reset input fields
        self.filter_state.reset_inputs();
        self.modal = Some(Modal::Filter);

        // Load schema for field suggestions
        if let Some(db) = &self.db {
            match db.infer_schema(&coll_name, 100).await {
                Ok(fields) => {
                    let field_names: Vec<String> = fields.into_iter().map(|f| f.name).collect();
                    self.filter_state.set_fields(field_names);
                }
                Err(e) => {
                    // Non-fatal - just no suggestions
                    self.filter_state.error = Some(format!("Schema betoltes hiba: {}", e));
                }
            }
        }
    }

    // execute_query törölve - használd execute_query_async

    // === Export ===

    /// Open export modal
    pub fn open_export_modal(&mut self) {
        let Some(coll) = self.collections.get(self.selected_collection) else {
            self.set_error("Nincs kiválasztott kollekció");
            return;
        };

        self.export_state = ExportState::new(coll.name.clone(), coll.doc_count.unwrap_or(0));
        self.modal = Some(Modal::Export);
    }

    // === New Collection ===

    /// Open new collection modal
    pub fn open_new_collection(&mut self) {
        self.new_collection_state = NewCollectionState::new();
        self.modal = Some(Modal::NewCollection);
    }

    // === IronRhai Scripts ===

    /// Open script modal (Browse mode)
    pub fn open_script_modal(&mut self) {
        self.script_state = ScriptState::new();
        self.modal = Some(Modal::Script);
        // Script list will be loaded in load_scripts_async
    }

    /// Load scripts from MCP server
    pub async fn load_scripts_async(&mut self) {
        let Some(db) = &self.db else {
            self.script_state.error = Some("Nincs megnyitva adatbázis".to_string());
            return;
        };

        match db.script_list().await {
            Ok(scripts_json) => {
                let scripts: Vec<ScriptInfo> = scripts_json
                    .iter()
                    .filter_map(|v| {
                        let name = v.get("name")?.as_str()?.to_string();
                        let description = v.get("description").and_then(|d| d.as_str()).map(String::from);
                        let version = v.get("version").and_then(|v| v.as_u64()).unwrap_or(1) as u32;
                        let tags = v.get("tags")
                            .and_then(|t| t.as_array())
                            .map(|arr| arr.iter().filter_map(|s| s.as_str().map(String::from)).collect())
                            .unwrap_or_default();
                        let execution_count = v.get("execution_count").and_then(|e| e.as_u64()).unwrap_or(0);
                        let last_run_at = v.get("last_run_at").and_then(|l| l.as_str()).map(String::from);
                        Some(ScriptInfo {
                            name,
                            description,
                            version,
                            tags,
                            execution_count,
                            last_run_at,
                        })
                    })
                    .collect();
                self.script_state.scripts = scripts;
                self.script_state.selected_script = 0;
                self.script_state.error = None;
            }
            Err(e) => {
                self.script_state.error = Some(format!("Script lista betöltése sikertelen: {}", e));
            }
        }
    }

    /// Load a script for editing by name
    pub async fn load_script_for_edit_async(&mut self) {
        let Some(script_info) = self.script_state.scripts.get(self.script_state.selected_script) else {
            return;
        };
        let script_name = script_info.name.clone();

        let Some(db) = &self.db else {
            self.script_state.error = Some("Nincs megnyitva adatbázis".to_string());
            return;
        };

        match db.script_get(&script_name).await {
            Ok(script_json) => {
                let code = script_json.get("code").and_then(|c| c.as_str()).unwrap_or("").to_string();
                let description = script_json.get("description").and_then(|d| d.as_str()).map(String::from);
                let version = script_json.get("version").and_then(|v| v.as_u64()).unwrap_or(1) as u32;
                let tags = script_json.get("tags")
                    .and_then(|t| t.as_array())
                    .map(|arr| arr.iter().filter_map(|s| s.as_str().map(String::from)).collect())
                    .unwrap_or_default();

                self.script_state.load_script(script_name, code, description, tags, version);
            }
            Err(e) => {
                self.script_state.error = Some(format!("Script betöltése sikertelen: {}", e));
            }
        }
    }

    /// Save current script to MCP server
    pub async fn save_script_async(&mut self) {
        if self.script_state.name.trim().is_empty() {
            self.script_state.error = Some("Script név megadása kötelező".to_string());
            return;
        }

        let Some(db) = &self.db else {
            self.script_state.error = Some("Nincs megnyitva adatbázis".to_string());
            return;
        };

        let code = self.script_state.lines.join("\n");
        let description = if self.script_state.description.is_empty() {
            None
        } else {
            Some(self.script_state.description.as_str())
        };
        let tags = if self.script_state.tags.is_empty() {
            None
        } else {
            Some(self.script_state.tags.as_slice())
        };

        match db.script_save(&self.script_state.name, &code, description, tags).await {
            Ok(result) => {
                let new_version = result.get("version").and_then(|v| v.as_u64()).unwrap_or(1) as u32;
                self.script_state.version = new_version;
                self.script_state.dirty = false;
                self.script_state.message = Some(format!("Script mentve (v{})", new_version));
                self.script_state.error = None;
            }
            Err(e) => {
                self.script_state.error = Some(format!("Mentés sikertelen: {}", e));
            }
        }
    }

    /// Delete selected script
    pub async fn delete_script_async(&mut self) {
        let Some(script_info) = self.script_state.scripts.get(self.script_state.selected_script) else {
            return;
        };
        let script_name = script_info.name.clone();

        let Some(db) = &self.db else {
            self.script_state.error = Some("Nincs megnyitva adatbázis".to_string());
            return;
        };

        match db.script_delete(&script_name).await {
            Ok(_) => {
                self.script_state.message = Some(format!("Script '{}' törölve", script_name));
                // Reload list
                self.load_scripts_async().await;
            }
            Err(e) => {
                self.script_state.error = Some(format!("Törlés sikertelen: {}", e));
            }
        }
    }

    /// Run script (saved or inline)
    pub async fn run_script_async(&mut self) {
        let Some(db) = &self.db else {
            self.script_state.error = Some("Nincs megnyitva adatbázis".to_string());
            return;
        };

        // Parse params
        let params: Option<serde_json::Value> = if self.script_state.params_input.trim().is_empty()
            || self.script_state.params_input.trim() == "{}"
        {
            None
        } else {
            match serde_json::from_str(&self.script_state.params_input) {
                Ok(v) => Some(v),
                Err(e) => {
                    self.script_state.error = Some(format!("Hibás JSON paraméterek: {}", e));
                    return;
                }
            }
        };

        let start = std::time::Instant::now();

        // Always execute current editor content (not saved version)
        // This ensures "what you see is what runs"
        let code = self.script_state.lines.join("\n");
        let result = db.script_exec(&code, params.as_ref()).await;

        let elapsed_ms = start.elapsed().as_millis() as u64;

        match result {
            Ok(result_json) => {
                let success = result_json.get("success").and_then(|s| s.as_bool()).unwrap_or(true);
                let output = result_json.get("result").cloned().unwrap_or(serde_json::Value::Null);
                let logs = result_json.get("logs")
                    .and_then(|l| l.as_array())
                    .map(|arr| arr.iter().filter_map(|s| s.as_str().map(String::from)).collect())
                    .unwrap_or_default();
                let error_msg = result_json.get("error").and_then(|e| e.as_str()).map(String::from);

                self.script_state.result = Some(ScriptResult {
                    success,
                    result: output,
                    logs,
                    error: error_msg,
                    execution_time_ms: elapsed_ms,
                });
                self.script_state.error = None;
            }
            Err(e) => {
                self.script_state.result = Some(ScriptResult {
                    success: false,
                    result: serde_json::Value::Null,
                    logs: vec![],
                    error: Some(e.to_string()),
                    execution_time_ms: elapsed_ms,
                });
            }
        }
    }

    /// Load script version history
    pub async fn load_script_history_async(&mut self) {
        let script_name = self.script_state.name.clone();
        if script_name.is_empty() || script_name == "[inline]" {
            self.script_state.error = Some("Nincs kiválasztva script".to_string());
            return;
        }

        let Some(db) = &self.db else {
            self.script_state.error = Some("Nincs megnyitva adatbázis".to_string());
            return;
        };

        match db.script_history(&script_name, Some(20)).await {
            Ok(history_json) => {
                let versions: Vec<ScriptVersion> = history_json
                    .iter()
                    .filter_map(|v| {
                        let version = v.get("version").and_then(|n| n.as_u64())? as u32;
                        let code = v.get("code").and_then(|c| c.as_str())?.to_string();
                        let description = v.get("description").and_then(|d| d.as_str()).map(String::from);
                        let created_at = v.get("created_at").and_then(|c| c.as_str()).unwrap_or("").to_string();
                        Some(ScriptVersion {
                            version,
                            code,
                            description,
                            created_at,
                        })
                    })
                    .collect();
                self.script_state.versions = versions;
                self.script_state.selected_version = 0;
                self.script_state.error = None;
            }
            Err(e) => {
                self.script_state.error = Some(format!("History betöltése sikertelen: {}", e));
            }
        }
    }

    /// Rollback script to a specific version
    pub async fn rollback_script_async(&mut self) {
        let Some(version_info) = self.script_state.versions.get(self.script_state.selected_version) else {
            self.script_state.error = Some("Nincs kiválasztva verzió".to_string());
            return;
        };
        let target_version = version_info.version;
        let script_name = self.script_state.name.clone();

        let Some(db) = &self.db else {
            self.script_state.error = Some("Nincs megnyitva adatbázis".to_string());
            return;
        };

        match db.script_rollback(&script_name, target_version).await {
            Ok(new_version) => {
                self.script_state.message = Some(format!("Rollback sikeres, új verzió: v{}", new_version));
                // Reload the script into editor
                self.load_script_for_edit_async().await;
            }
            Err(e) => {
                self.script_state.error = Some(format!("Rollback sikertelen: {}", e));
            }
        }
    }

    /// Create new collection (async)
    pub async fn create_collection_async(&mut self) {
        let name = self.new_collection_state.name.trim().to_string();
        if name.is_empty() {
            self.new_collection_state.error = Some("Név megadása kötelező".to_string());
            return;
        }

        // Check if collection already exists
        if self.collections.iter().any(|c| c.name == name) {
            self.new_collection_state.error = Some("Ez a kollekció már létezik".to_string());
            return;
        }

        let Some(db) = &self.db else {
            self.set_error("Nincs megnyitva adatbázis");
            return;
        };

        match db.create_collection(&name).await {
            Ok(()) => {
                self.close_modal();
                self.set_status(format!("Kollekció létrehozva: {}", name));
                let _ = self.refresh_collections_async().await;
                // Select the new collection
                if let Some(idx) = self.collections.iter().position(|c| c.name == name) {
                    self.selected_collection = idx;
                    let _ = self.refresh_documents_async().await;
                }
            }
            Err(e) => {
                self.new_collection_state.error = Some(format!("Hiba: {}", e));
            }
        }
    }

    // === Theme ===

    /// Cycle to next theme
    pub fn next_theme(&mut self) {
        self.theme_name = self.theme_name.next();
        self.theme = Theme::from_name(self.theme_name);
        self.status_message = Some(format!("Tema: {}", self.theme_name.name()));
    }

    // === Status ===

    /// Set status message
    pub fn set_status(&mut self, msg: impl Into<String>) {
        self.status_message = Some(msg.into());
    }

    /// Set error message
    pub fn set_error(&mut self, msg: impl Into<String>) {
        self.error_message = Some(msg.into());
    }

    /// Clear error
    pub fn clear_error(&mut self) {
        self.error_message = None;
    }

    /// Reset UI state for database switch (clear documents, filters, selections)
    pub fn reset_for_new_database(&mut self) {
        self.documents.clear();
        self.selected_collection = 0;
        self.selected_document = 0;
        self.doc_scroll_offset = 0;
        self.detail_scroll = 0;
        self.total_docs = 0;
        self.active_filter = None;
        self.active_sort = None;
    }

    /// Copy selected document JSON to clipboard
    pub fn copy_document_to_clipboard(&mut self) {
        let Some(doc) = self.get_selected_document() else {
            self.set_error("Nincs kiválasztott dokumentum");
            return;
        };

        let json = serde_json::to_string_pretty(doc).unwrap_or_else(|_| "{}".to_string());

        match arboard::Clipboard::new() {
            Ok(mut clipboard) => match clipboard.set_text(&json) {
                Ok(()) => {
                    let preview: String = json.chars().take(50).collect();
                    self.set_status(format!("Vágólapra másolva: {}...", preview));
                }
                Err(e) => {
                    self.set_error(format!("Clipboard hiba: {}", e));
                }
            },
            Err(e) => {
                self.set_error(format!("Clipboard nem elérhető: {}", e));
            }
        }
    }

    /// Get database name
    pub fn db_name(&self) -> &str {
        self.db_path
            .as_ref()
            .and_then(|p| p.file_stem())
            .and_then(|s| s.to_str())
            .unwrap_or("(nincs)")
    }

    /// Get total doc count across all loaded collections
    pub fn total_doc_count(&self) -> usize {
        self.collections.iter().filter_map(|c| c.doc_count).sum()
    }

    // ============================================
    // === Async methods for MCP communication ===
    // ============================================

    /// Fetch database path from MCP server (for HTTP mode)
    /// This sets app.db_path from db_stats response
    pub async fn fetch_db_path_async(&mut self) -> anyhow::Result<()> {
        if let Some(db) = &self.db {
            let stats = db.db_stats().await?;
            if let Some(path_str) = stats.get("database_path").and_then(|v| v.as_str()) {
                self.db_path = Some(std::path::PathBuf::from(path_str));
            }
        }
        Ok(())
    }

    /// Refresh collections list (async)
    pub async fn refresh_collections_async(&mut self) -> anyhow::Result<()> {
        self.refresh_collections_inner().await
    }

    async fn refresh_collections_inner(&mut self) -> anyhow::Result<()> {
        if let Some(db) = &self.db {
            let collections = db.list_collections().await?;
            self.collections = collections;
            if !self.collections.is_empty() && self.selected_collection >= self.collections.len() {
                self.selected_collection = 0;
            }
            self.refresh_documents_inner().await?;
        }
        Ok(())
    }

    /// Refresh documents for selected collection (async)
    pub async fn refresh_documents_async(&mut self) -> anyhow::Result<()> {
        let coll_name = self
            .collections
            .get(self.selected_collection)
            .map(|c| c.name.clone())
            .unwrap_or_default();
        if coll_name.is_empty() {
            return Ok(());
        }
        self.refresh_documents_inner().await
    }

    async fn refresh_documents_inner(&mut self) -> anyhow::Result<()> {
        let (coll_name, needs_details) = match self.collections.get(self.selected_collection) {
            Some(coll) => (coll.name.clone(), !coll.is_loaded()),
            None => return Ok(()),
        };

        if let Some(db) = &self.db {
            let skip = self.doc_scroll_offset;

            // Use active filter/sort if set
            let query = self.active_filter.clone().unwrap_or(serde_json::json!({}));
            let docs = db
                .find_with_sort(
                    &coll_name,
                    &query,
                    skip,
                    self.page_size,
                    self.active_sort.as_ref(),
                )
                .await?;
            self.documents = docs;

            // Lazy load: only fetch details if not already loaded (skip if filtering)
            if needs_details && self.active_filter.is_none() {
                let (doc_count, index_names) = db.load_collection_details(&coll_name).await?;
                self.total_docs = doc_count;

                // Update cached collection info
                if let Some(coll) = self.collections.get_mut(self.selected_collection) {
                    coll.doc_count = Some(doc_count);
                    coll.index_count = Some(index_names.len());
                }

                // Update index_state for UI display
                self.index_state.indexes = index_names;
            } else if self.active_filter.is_none() {
                // Use cached count ONLY when not filtering
                // (filtered total_docs is set by execute_filter_async)
                if let Some(coll) = self.collections.get(self.selected_collection) {
                    self.total_docs = coll.doc_count.unwrap_or(0);
                }
            }
            // When active_filter is set, total_docs is preserved from execute_filter_async
        }
        Ok(())
    }

    /// Get indexes for current collection (async)
    pub async fn get_current_indexes_async(&self) -> Vec<String> {
        if let Some(db) = &self.db {
            if let Some(coll) = self.collections.get(self.selected_collection) {
                return db.list_indexes(&coll.name).await.unwrap_or_default();
            }
        }
        Vec::new()
    }

    // === Async Navigation ===

    /// Move selection up in current pane (async)
    pub async fn select_up_async(&mut self) {
        match self.active_pane {
            Pane::Collections => {
                if self.selected_collection > 0 {
                    self.selected_collection -= 1;
                    self.doc_scroll_offset = 0;
                    self.selected_document = 0;
                    self.active_filter = None; // Clear filter on collection change
                    self.active_sort = None; // Clear sort on collection change
                    self.filter_state = FilterState::new(); // Clear filter state too
                    let _ = self.refresh_documents_async().await;
                }
            }
            Pane::Documents => {
                if self.selected_document > 0 {
                    self.selected_document -= 1;
                    if self.selected_document < self.doc_scroll_offset {
                        self.doc_scroll_offset = self.selected_document;
                        let _ = self.refresh_documents_async().await;
                    }
                }
            }
            Pane::Detail => {
                if self.detail_scroll > 0 {
                    self.detail_scroll -= 1;
                }
            }
        }
    }

    /// Move selection down in current pane (async)
    pub async fn select_down_async(&mut self) {
        match self.active_pane {
            Pane::Collections => {
                if self.selected_collection + 1 < self.collections.len() {
                    self.selected_collection += 1;
                    self.doc_scroll_offset = 0;
                    self.selected_document = 0;
                    self.active_filter = None; // Clear filter on collection change
                    self.active_sort = None; // Clear sort on collection change
                    self.filter_state = FilterState::new(); // Clear filter state too
                    let _ = self.refresh_documents_async().await;
                }
            }
            Pane::Documents => {
                if self.selected_document + 1 < self.total_docs {
                    self.selected_document += 1;
                    let visible_end = self.doc_scroll_offset + self.page_size;
                    if self.selected_document >= visible_end {
                        self.doc_scroll_offset += 1;
                        let _ = self.refresh_documents_async().await;
                    }
                }
            }
            Pane::Detail => {
                // Allow free scrolling - rendering handles bounds
                self.detail_scroll += 1;
            }
        }
    }

    /// Page up (async)
    pub async fn page_up_async(&mut self) {
        match self.active_pane {
            Pane::Collections => {
                self.selected_collection = self.selected_collection.saturating_sub(SCROLL_STEP);
                self.doc_scroll_offset = 0;
                self.selected_document = 0;
                let _ = self.refresh_documents_async().await;
            }
            Pane::Documents => {
                let old_offset = self.doc_scroll_offset;
                self.doc_scroll_offset = self.doc_scroll_offset.saturating_sub(self.page_size);
                self.selected_document = self.selected_document.saturating_sub(self.page_size);
                if old_offset != self.doc_scroll_offset {
                    let _ = self.refresh_documents_async().await;
                }
            }
            Pane::Detail => {
                self.detail_scroll = self.detail_scroll.saturating_sub(SCROLL_STEP);
            }
        }
    }

    /// Page down (async)
    pub async fn page_down_async(&mut self) {
        match self.active_pane {
            Pane::Collections => {
                let max = self.collections.len().saturating_sub(1);
                self.selected_collection = (self.selected_collection + SCROLL_STEP).min(max);
                self.doc_scroll_offset = 0;
                self.selected_document = 0;
                let _ = self.refresh_documents_async().await;
            }
            Pane::Documents => {
                let max_offset = self.total_docs.saturating_sub(self.page_size);
                let old_offset = self.doc_scroll_offset;
                self.doc_scroll_offset = (self.doc_scroll_offset + self.page_size).min(max_offset);
                self.selected_document = (self.selected_document + self.page_size)
                    .min(self.total_docs.saturating_sub(1));
                if old_offset != self.doc_scroll_offset {
                    let _ = self.refresh_documents_async().await;
                }
            }
            Pane::Detail => {
                // Allow free scrolling - rendering handles bounds
                self.detail_scroll += SCROLL_STEP;
            }
        }
    }

    /// Go to start (async)
    pub async fn go_to_start_async(&mut self) {
        match self.active_pane {
            Pane::Collections => {
                if self.selected_collection != 0 {
                    self.selected_collection = 0;
                    self.doc_scroll_offset = 0;
                    self.selected_document = 0;
                    let _ = self.refresh_documents_async().await;
                }
            }
            Pane::Documents => {
                if self.doc_scroll_offset != 0 || self.selected_document != 0 {
                    self.doc_scroll_offset = 0;
                    self.selected_document = 0;
                    let _ = self.refresh_documents_async().await;
                }
            }
            Pane::Detail => {
                self.detail_scroll = 0;
            }
        }
    }

    /// Go to end (async)
    pub async fn go_to_end_async(&mut self) {
        match self.active_pane {
            Pane::Collections => {
                if self.collections.is_empty() {
                    return;
                }
                let last = self.collections.len() - 1;
                if self.selected_collection != last {
                    self.selected_collection = last;
                    self.doc_scroll_offset = 0;
                    self.selected_document = 0;
                    let _ = self.refresh_documents_async().await;
                }
            }
            Pane::Documents => {
                if self.total_docs == 0 {
                    return;
                }
                let last = self.total_docs - 1;
                self.selected_document = last;
                self.doc_scroll_offset = last.saturating_sub(self.page_size.saturating_sub(1));
                let _ = self.refresh_documents_async().await;
            }
            Pane::Detail => {
                // Scroll to end - use total lines as scroll position
                // Rendering will show last lines at top of view
                self.detail_scroll = self.get_selected_document_lines();
            }
        }
    }

    // === Async Search ===

    /// Execute search based on current mode and query (async)
    pub async fn execute_search_async(&mut self) {
        match self.search.mode {
            SearchMode::Collections => self.execute_collection_search_async().await,
            SearchMode::Document => self.execute_document_search(),
        }
    }

    /// Search collections (async - requires DB call)
    async fn execute_collection_search_async(&mut self) {
        let query = self.search.query_input.trim();
        if query.is_empty() {
            self.search.results.clear();
            return;
        }

        if query == self.search.last_search && !self.search.results.is_empty() {
            return;
        }

        self.search.last_search = query.to_string();
        self.search.results.clear();
        self.search.selected_result = 0;
        self.search.result_offset = 0;

        let Some(db) = &self.db else {
            self.set_error("Nincs megnyitva adatbazis");
            return;
        };

        match db.search_collections(query).await {
            Ok(collections) => {
                self.search.results = collections
                    .into_iter()
                    .map(|c| SearchResult {
                        name: c.name,
                        doc_count: c.doc_count.unwrap_or(0),
                    })
                    .collect();
            }
            Err(e) => {
                self.set_error(format!("Kereses hiba: {}", e));
            }
        }

        if self.search.results.is_empty() {
            self.set_status("Nincs talalat");
        } else {
            self.set_status(format!("{} talalat", self.search.results.len()));
        }
    }

    /// Search within current document (sync - local operation)
    pub fn execute_document_search(&mut self) {
        let query = self.search.query_input.trim().to_lowercase();
        if query.is_empty() {
            self.search.doc_matches.clear();
            return;
        }

        self.search.doc_matches.clear();
        self.search.current_match = 0;

        // Get current document
        let Some(doc) = self.get_selected_document() else {
            self.set_status("Nincs kivalasztott dokumentum");
            return;
        };

        // Pretty print and search
        let pretty = serde_json::to_string_pretty(&doc).unwrap_or_default();

        for (line_num, line) in pretty.lines().enumerate() {
            let line_lower = line.to_lowercase();
            let mut search_start = 0;
            while let Some(col) = line_lower[search_start..].find(&query) {
                self.search.doc_matches.push(DocSearchMatch {
                    line: line_num,
                    col_start: search_start + col,
                });
                search_start += col + query.len();
            }
        }

        if self.search.doc_matches.is_empty() {
            self.set_status("Nincs talalat");
        } else {
            self.set_status(format!(
                "{} talalat (n/N: kovetkezo/elozo)",
                self.search.doc_matches.len()
            ));
            // Jump to first match
            self.scroll_to_current_match();
        }
    }

    /// Scroll detail pane to show current match
    pub fn scroll_to_current_match(&mut self) {
        if let Some(line) = self.search.current_match_line() {
            // Leave some context lines above
            self.detail_scroll = line.saturating_sub(3);
        }
    }

    /// Navigate to next/prev match and scroll
    pub fn goto_next_match(&mut self) {
        self.search.next_match();
        self.scroll_to_current_match();
        if !self.search.doc_matches.is_empty() {
            self.set_status(format!(
                "Talalat {}/{}",
                self.search.current_match + 1,
                self.search.doc_matches.len()
            ));
        }
    }

    pub fn goto_prev_match(&mut self) {
        self.search.prev_match();
        self.scroll_to_current_match();
        if !self.search.doc_matches.is_empty() {
            self.set_status(format!(
                "Talalat {}/{}",
                self.search.current_match + 1,
                self.search.doc_matches.len()
            ));
        }
    }

    /// Navigate to selected search result (async)
    pub async fn goto_search_result_async(&mut self) {
        if let Some(result) = self
            .search
            .results
            .get(self.search.selected_result)
            .cloned()
        {
            // Navigate to collection
            if let Some(idx) = self.collections.iter().position(|c| c.name == result.name) {
                self.selected_collection = idx;
                self.doc_scroll_offset = 0;
                self.selected_document = 0;
                let _ = self.refresh_documents_async().await;
                self.active_pane = Pane::Documents;
            }
            self.close_modal();
        }
    }

    // === Async Confirm ===

    /// Execute the confirmed action (async)
    pub async fn execute_confirm_action_async(&mut self) {
        if self.confirm.selected != ConfirmOption::Confirm {
            self.close_modal();
            return;
        }

        let action = self.confirm.on_confirm.clone();
        self.close_modal();

        match action {
            ConfirmAction::DeleteDocument { collection, doc_id } => {
                self.do_delete_document_async(&collection, &doc_id).await;
            }
            ConfirmAction::DeleteCollection { name } => {
                self.do_delete_collection_async(&name).await;
            }
        }
    }

    /// Actually delete a document (async)
    async fn do_delete_document_async(&mut self, collection: &str, doc_id: &Value) {
        let Some(db) = &self.db else {
            self.set_error("Nincs megnyitva adatbázis");
            return;
        };

        match db.delete_document(collection, doc_id).await {
            Ok(count) => {
                if count > 0 {
                    self.set_status("Dokumentum törölve");
                    let _ = self.refresh_collections_async().await;
                } else {
                    self.set_error("Dokumentum nem található");
                }
            }
            Err(e) => {
                self.set_error(format!("Törlés hiba: {}", e));
            }
        }
    }

    /// Actually delete a collection (async)
    async fn do_delete_collection_async(&mut self, name: &str) {
        let Some(db) = &self.db else {
            self.set_error("Nincs megnyitva adatbázis");
            return;
        };

        match db.drop_collection(name).await {
            Ok(()) => {
                self.set_status(format!("Kollekció törölve: {}", name));
                // Reset selection and refresh
                self.selected_collection = 0;
                self.documents.clear();
                self.selected_document = 0;
                self.doc_scroll_offset = 0;
                let _ = self.refresh_collections_async().await;
            }
            Err(e) => {
                self.set_error(format!("Törlés hiba: {}", e));
            }
        }
    }

    // === Async Insert/Edit ===

    /// Execute the insert/edit operation (async)
    pub async fn execute_insert_async(&mut self) {
        let doc = match self.insert.parse_json() {
            Ok(doc) => doc,
            Err(e) => {
                self.insert.error = Some(format!("JSON hiba: {}", e));
                return;
            }
        };

        if !doc.is_object() {
            self.insert.error = Some("A dokumentumnak objektumnak kell lennie".to_string());
            return;
        }

        let collection = self.insert.collection.clone();
        let Some(db) = &self.db else {
            self.set_error("Nincs megnyitva adatbázis");
            return;
        };

        if self.insert.is_edit_mode() {
            let Some(ref doc_id) = self.insert.original_doc_id else {
                self.insert.error = Some("Eredeti dokumentum ID hiányzik".to_string());
                return;
            };

            let update = serde_json::json!({"$set": doc});
            match db.update_document(&collection, doc_id, &update).await {
                Ok(count) => {
                    self.close_modal();
                    if count > 0 {
                        self.set_status("Dokumentum frissítve");
                    } else {
                        self.set_status("Dokumentum nem található");
                    }
                    let _ = self.refresh_collections_async().await;
                }
                Err(e) => {
                    self.insert.error = Some(format!("Frissítés hiba: {}", e));
                }
            }
        } else {
            match db.insert_document(&collection, &doc).await {
                Ok(id) => {
                    self.close_modal();
                    self.set_status(format!("Dokumentum beszúrva, _id: {}", id));
                    let _ = self.refresh_collections_async().await;
                }
                Err(e) => {
                    self.insert.error = Some(format!("Beszúrás hiba: {}", e));
                }
            }
        }
    }

    // === Async Index Management ===

    /// Create a new index (async)
    pub async fn execute_create_index_async(&mut self) {
        let field = self.index_state.field_input.trim().to_string();
        if field.is_empty() {
            self.index_state.error = Some("Mező neve kötelező".to_string());
            return;
        }

        let collection = self.index_state.collection.clone();
        let unique = self.index_state.unique;

        let Some(db) = &self.db else {
            self.set_error("Nincs megnyitva adatbázis");
            return;
        };

        match db.create_index(&collection, &field, unique).await {
            Ok(()) => {
                self.index_state.message = Some(format!("Index létrehozva: {}", field));
                self.index_state.cancel_create();
                self.index_state.indexes = self.get_current_indexes_async().await;
            }
            Err(e) => {
                self.index_state.error = Some(format!("Hiba: {}", e));
            }
        }
    }

    /// Delete selected index (async)
    pub async fn execute_delete_index_async(&mut self) {
        if self.index_state.indexes.is_empty() {
            return;
        }

        let index_name = self.index_state.indexes[self.index_state.selected_index].clone();
        let collection = self.index_state.collection.clone();

        let Some(db) = &self.db else {
            self.set_error("Nincs megnyitva adatbázis");
            return;
        };

        match db.drop_index(&collection, &index_name).await {
            Ok(()) => {
                self.index_state.message = Some(format!("Index törölve: {}", index_name));
                self.index_state.indexes = self.get_current_indexes_async().await;
                // Adjust selection after deletion
                if self.index_state.indexes.is_empty() {
                    self.index_state.selected_index = 0;
                } else if self.index_state.selected_index >= self.index_state.indexes.len() {
                    self.index_state.selected_index = self.index_state.indexes.len() - 1;
                }
            }
            Err(e) => {
                self.index_state.error = Some(format!("Hiba: {}", e));
            }
        }
    }

    // === Async Query ===

    /// Execute the query (async) - max 1000 results
    pub async fn execute_query_async(&mut self) {
        const QUERY_LIMIT: usize = 1000;

        let query = match self.query_state.parse_query() {
            Ok(q) => q,
            Err(e) => {
                self.query_state.error = Some(format!("JSON hiba: {}", e));
                return;
            }
        };

        let collection = self.query_state.collection.clone();

        let Some(db) = &self.db else {
            self.set_error("Nincs megnyitva adatbázis");
            return;
        };

        match db
            .find_with_options(&collection, &query, 0, QUERY_LIMIT)
            .await
        {
            Ok(docs) => {
                let count = docs.len();
                // Store total_count (may be more than returned if limit hit)
                self.query_state.total_count = count;
                self.query_state.result_index = 0;
                self.query_state.results = Some(docs);
                self.query_state.error = None;
                if count >= QUERY_LIMIT {
                    self.set_status(format!("{}+ dokumentum (limit: {})", count, QUERY_LIMIT));
                } else {
                    self.set_status(format!("{} dokumentum találva", count));
                }
            }
            Err(e) => {
                self.query_state.error = Some(format!("Lekérdezés hiba: {}", e));
            }
        }
    }

    /// Apply the current query as a filter and close modal
    pub async fn apply_query_as_filter(&mut self) {
        let query = match self.query_state.parse_query() {
            Ok(q) => q,
            Err(e) => {
                self.query_state.error = Some(format!("JSON hiba: {}", e));
                return;
            }
        };

        // Set active filter
        self.active_filter = Some(query);
        self.selected_document = 0;
        self.doc_scroll_offset = 0;

        // Close modal
        self.close_modal();

        // Refresh documents with filter
        let _ = self.refresh_documents_async().await;
    }

    // === Async Filter ===

    /// Execute the visual filter (async) - max 1000 results
    pub async fn execute_filter_async(&mut self) {
        const FILTER_LIMIT: usize = 1000;

        // Ha van kitöltött field, automatikusan adjuk hozzá keresés előtt
        if !self.filter_state.field_input.is_empty() {
            self.filter_state.add_filter();
        }

        if self.filter_state.filters.is_empty() {
            self.filter_state.error = Some("Adj hozza legalabb egy szurot".to_string());
            return;
        }

        let query = self.filter_state.build_query();
        let collection = match self.current_collection_name() {
            Some(name) => name.to_string(),
            None => {
                self.filter_state.error = Some("Nincs kiválasztott kollekció".to_string());
                return;
            }
        };

        // Build sort from filter state
        let sort = self.filter_state.build_sort();

        let Some(db) = &self.db else {
            self.filter_state.error = Some("Nincs megnyitva adatbázis".to_string());
            return;
        };

        // First get the total count matching the query
        let total_count = match db.count_with_query(&collection, &query).await {
            Ok(c) => c,
            Err(e) => {
                self.filter_state.error = Some(format!("Szamlalas hiba: {}", e));
                return;
            }
        };

        if total_count == 0 {
            // 0 eredmény - maradjon nyitva a modal, mutassa a query-t
            self.filter_state.result_count = 0;
            self.filter_state.results = Some(vec![]);
            self.filter_state.error = Some(format!(
                "0 talalat. Query: {}",
                serde_json::to_string(&query).unwrap_or_default()
            ));
            return;
        }

        // Load first page of results
        match db
            .find_with_sort(&collection, &query, 0, self.page_size, sort.as_ref())
            .await
        {
            Ok(docs) => {
                self.filter_state.result_count = total_count;
                self.filter_state.results = Some(docs.clone());
                self.filter_state.error = None;

                // Van eredmény - frissítsd a dokumentum listát és zárd be
                self.documents = docs;
                self.selected_document = 0;
                self.doc_scroll_offset = 0;
                self.total_docs = total_count; // Use actual total, not loaded count!
                self.active_filter = Some(query.clone()); // Store filter for pagination
                self.active_sort = sort; // Store sort for pagination

                self.set_status(format!("{} talalat", total_count));

                // Close the modal to show results
                self.close_modal();
            }
            Err(e) => {
                self.filter_state.error = Some(format!("Szures hiba: {}", e));
            }
        }
    }

    // === Async Export ===

    /// Execute the export (async) - streaming with batches
    pub async fn execute_export_async(&mut self) {
        use crate::modals::export::ExportFormat;
        use std::fs::File;
        use std::io::{BufWriter, Write};

        const BATCH_SIZE: usize = 1000;

        if self.export_state.file_path.trim().is_empty() {
            self.export_state.error = Some("Fájl útvonal megadása kötelező".to_string());
            return;
        }

        let collection = self.export_state.collection.clone();
        let file_path = self.export_state.file_path.clone();
        let format = self.export_state.format;
        let total_docs = self.export_state.doc_count;

        let file = match File::create(&file_path) {
            Ok(f) => f,
            Err(e) => {
                self.export_state.error = Some(format!("Fájl hiba: {}", e));
                return;
            }
        };
        let mut writer = BufWriter::new(file);

        let Some(db) = &self.db else {
            self.set_error("Nincs megnyitva adatbázis");
            return;
        };

        let result: Result<usize, String> = match format {
            ExportFormat::Json => {
                if let Err(e) = writeln!(writer, "[") {
                    return self.export_state.error = Some(format!("Írási hiba: {}", e));
                }

                let mut exported = 0;
                let mut first = true;
                let mut offset = 0;

                loop {
                    let docs = match db
                        .find_with_options(&collection, &serde_json::json!({}), offset, BATCH_SIZE)
                        .await
                    {
                        Ok(d) => d,
                        Err(e) => {
                            self.export_state.error = Some(format!("Lekérdezés hiba: {}", e));
                            return;
                        }
                    };

                    if docs.is_empty() {
                        break;
                    }

                    for doc in &docs {
                        let prefix = if first { "  " } else { ",\n  " };
                        first = false;

                        let json = match serde_json::to_string(doc) {
                            Ok(j) => j,
                            Err(e) => {
                                self.export_state.error = Some(format!("JSON hiba: {}", e));
                                return;
                            }
                        };

                        if let Err(e) = write!(writer, "{}{}", prefix, json) {
                            self.export_state.error = Some(format!("Írási hiba: {}", e));
                            return;
                        }
                    }

                    exported += docs.len();
                    offset += docs.len();
                }

                if let Err(e) = writeln!(writer, "\n]") {
                    self.export_state.error = Some(format!("Írási hiba: {}", e));
                    return;
                }

                Ok(exported)
            }
            ExportFormat::Csv => {
                let sample = match db
                    .find_with_options(&collection, &serde_json::json!({}), 0, 100)
                    .await
                {
                    Ok(d) => d,
                    Err(e) => {
                        self.export_state.error = Some(format!("Lekérdezés hiba: {}", e));
                        return;
                    }
                };

                if sample.is_empty() {
                    self.export_state.error = Some("Nincs exportálandó dokumentum".to_string());
                    return;
                }

                let mut fields: Vec<String> = Vec::new();
                for doc in &sample {
                    if let Some(obj) = doc.as_object() {
                        for key in obj.keys() {
                            if !fields.contains(key) {
                                fields.push(key.clone());
                            }
                        }
                    }
                }

                if let Err(e) = writeln!(writer, "{}", fields.join(",")) {
                    self.export_state.error = Some(format!("Írási hiba: {}", e));
                    return;
                }

                let mut exported = 0;
                let mut offset = 0;

                loop {
                    let docs = match db
                        .find_with_options(&collection, &serde_json::json!({}), offset, BATCH_SIZE)
                        .await
                    {
                        Ok(d) => d,
                        Err(e) => {
                            self.export_state.error = Some(format!("Lekérdezés hiba: {}", e));
                            return;
                        }
                    };

                    if docs.is_empty() {
                        break;
                    }

                    for doc in &docs {
                        let row: Vec<String> = fields
                            .iter()
                            .map(|f| {
                                doc.get(f)
                                    .map(|v| {
                                        if v.is_string() {
                                            let s = v.as_str().unwrap_or("");
                                            format!("\"{}\"", s.replace('"', "\"\""))
                                        } else if v.is_null() {
                                            String::new()
                                        } else {
                                            v.to_string()
                                        }
                                    })
                                    .unwrap_or_default()
                            })
                            .collect();

                        if let Err(e) = writeln!(writer, "{}", row.join(",")) {
                            self.export_state.error = Some(format!("Írási hiba: {}", e));
                            return;
                        }
                    }

                    exported += docs.len();
                    offset += docs.len();
                }

                Ok(exported)
            }
        };

        if let Err(e) = writer.flush() {
            self.export_state.error = Some(format!("Flush hiba: {}", e));
            return;
        }

        match result {
            Ok(count) => {
                self.export_state.message =
                    Some(format!("{} dokumentum exportálva: {}", count, file_path));
                self.export_state.error = None;
            }
            Err(e) => {
                self.export_state.error = Some(format!("Export hiba: {}", e));
            }
        }
    }
}
