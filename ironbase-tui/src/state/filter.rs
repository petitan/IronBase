//! Filter state for visual query building

use serde_json::Value;

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

    #[allow(clippy::wrong_self_convention)]
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

    #[allow(clippy::wrong_self_convention)]
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
            FilterFocus::Field if self.field_cursor > 0 => {
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
            FilterFocus::Value if self.value_cursor > 0 => {
                self.value_cursor -= 1;
                let byte_pos = self
                    .value_input
                    .char_indices()
                    .nth(self.value_cursor)
                    .map(|(i, _)| i)
                    .unwrap_or(0);
                self.value_input.remove(byte_pos);
            }
            _ => {}
        }
    }

    pub fn cursor_left(&mut self) {
        match self.focus {
            FilterFocus::Field if self.field_cursor > 0 => {
                self.field_cursor -= 1;
            }
            FilterFocus::Value if self.value_cursor > 0 => {
                self.value_cursor -= 1;
            }
            _ => {}
        }
    }

    pub fn cursor_right(&mut self) {
        match self.focus {
            FilterFocus::Field
                // Use character count, not byte length
                if self.field_cursor < self.field_input.chars().count() => {
                    self.field_cursor += 1;
                }
            FilterFocus::Value
                if self.value_cursor < self.value_input.chars().count() => {
                    self.value_cursor += 1;
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
