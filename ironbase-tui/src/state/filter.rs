//! Filter state for visual query building

use crate::widgets::{Input, InputOutcome, TextInput};
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

    /// The operator as the UI shows it (in the UI language)
    pub fn label(&self) -> &'static str {
        crate::i18n::t().filter_operator(*self)
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

    /// The direction as the UI shows it (in the UI language)
    pub fn label(&self) -> &'static str {
        crate::i18n::t().sort_direction(*self)
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
    pub field_input: TextInput,
    pub operator: FilterOperator,
    pub operator_idx: usize,
    pub value_input: TextInput,
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
        self.value_input.clear();
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
        // The schema is reloaded on every open: clamp a stale index
        let current = self.sort_field_idx.min(self.all_fields.len());
        self.sort_field_idx = (current + 1) % (self.all_fields.len() + 1);
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
        // The schema is reloaded on every open: clamp a stale index
        let current = self.sort_field_idx.min(self.all_fields.len());
        self.sort_field_idx = if current == 0 { total - 1 } else { current - 1 };
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
            let condition = FilterCondition {
                field: self.field_input.value().to_string(),
                operator: self.operator,
                value: self.value_input.value().to_string(),
            };
            self.filters.push(condition);
            self.field_input.clear();
            self.value_input.clear();
            self.operator = FilterOperator::default();
            self.operator_idx = 0;
            self.focus = FilterFocus::Field;
            // The suggestions follow the (now empty) field input
            self.update_suggestions();
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
            self.field_input.set_value(filter.field);
            self.value_input.set_value(filter.value);
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

    /// Edit the focused text input (field name or value); the suggestions
    /// follow the field name
    pub fn handle_field_input(&mut self, input: Input) -> InputOutcome {
        match self.focus {
            FilterFocus::Field => {
                let outcome = self.field_input.apply(input);
                if outcome == InputOutcome::Changed {
                    self.update_suggestions();
                }
                outcome
            }
            FilterFocus::Value => self.value_input.apply(input),
            _ => InputOutcome::Ignored,
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
        let input = self.field_input.value().to_lowercase();
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
            self.field_input.set_value(suggestion.clone());
            self.show_suggestions = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyModifiers};
    use serde_json::json;

    fn key(state: &mut FilterState, code: KeyCode) -> InputOutcome {
        state.handle_field_input(Input::Key(code, KeyModifiers::NONE))
    }

    fn type_into(state: &mut FilterState, text: &str) {
        for c in text.chars() {
            key(state, KeyCode::Char(c));
        }
    }

    fn condition(field: &str, operator: FilterOperator, value: &str) -> FilterCondition {
        FilterCondition {
            field: field.to_string(),
            operator,
            value: value.to_string(),
        }
    }

    #[test]
    fn test_query_per_operator() {
        use FilterOperator::*;
        let q = |op: FilterOperator, value: &str| op.to_query("f", value);

        assert_eq!(q(Equals, "x"), json!({"f": "x"}));
        assert_eq!(q(NotEquals, "x"), json!({"f": {"$ne": "x"}}));
        assert_eq!(q(GreaterThan, "5"), json!({"f": {"$gt": 5}}));
        assert_eq!(q(LessThan, "5"), json!({"f": {"$lt": 5}}));
        assert_eq!(q(GreaterOrEq, "5"), json!({"f": {"$gte": 5}}));
        assert_eq!(q(LessOrEq, "5"), json!({"f": {"$lte": 5}}));
        // Regex operators take the text as typed: no escaping, no options
        assert_eq!(q(Contains, "a.b"), json!({"f": {"$regex": "a.b"}}));
        assert_eq!(q(StartsWith, "12"), json!({"f": {"$regex": "^12"}}));
        // Exists: anything but "false" / "0" means true (also empty)
        assert_eq!(q(Exists, ""), json!({"f": {"$exists": true}}));
        assert_eq!(q(Exists, "true"), json!({"f": {"$exists": true}}));
        assert_eq!(q(Exists, "false"), json!({"f": {"$exists": false}}));
        assert_eq!(q(Exists, "0"), json!({"f": {"$exists": false}}));
        // In: comma separated, trimmed, empty items skipped, each value guessed
        assert_eq!(
            q(In, "a, 2 ,,true, 1.5"),
            json!({"f": {"$in": ["a", 2, true, 1.5]}})
        );
        assert_eq!(q(In, " , "), json!({"f": {"$in": []}}));
    }

    /// The value type is guessed from the text: integer, float, bool, null,
    /// otherwise string (there is no way to ask for the string "5")
    #[test]
    fn test_value_type_guessing() {
        let q = |value: &str| FilterOperator::Equals.to_query("f", value)["f"].clone();

        assert_eq!(q("42"), json!(42));
        assert_eq!(q("-7"), json!(-7));
        assert_eq!(q("+7"), json!(7));
        assert_eq!(q("007"), json!(7));
        assert_eq!(q("1.5"), json!(1.5));
        assert_eq!(q("1e3"), json!(1000.0));
        assert_eq!(q("true"), json!(true));
        assert_eq!(q("false"), json!(false));
        assert_eq!(q("null"), json!(null));
        assert_eq!(q("True"), json!("True"));
        assert_eq!(q("abc"), json!("abc"));
        assert_eq!(q(""), json!(""));
        assert_eq!(q(" 5"), json!(" 5"));
        assert_eq!(q("árvíz"), json!("árvíz"));
    }

    #[test]
    fn test_build_query_combines_conditions_with_and() {
        let mut state = FilterState::new();
        assert_eq!(state.build_query(), json!({}));

        state
            .filters
            .push(condition("age", FilterOperator::GreaterThan, "30"));
        assert_eq!(state.build_query(), json!({"age": {"$gt": 30}}));

        state
            .filters
            .push(condition("city", FilterOperator::Equals, "Győr"));
        assert_eq!(
            state.build_query(),
            json!({"$and": [{"age": {"$gt": 30}}, {"city": "Győr"}]})
        );
    }

    #[test]
    fn test_condition_display() {
        assert_eq!(
            condition("age", FilterOperator::GreaterOrEq, "30").display(),
            "age >= \"30\""
        );
        assert_eq!(
            condition("email", FilterOperator::Exists, "").display(),
            "email létezik"
        );
    }

    #[test]
    fn test_sort_spec() {
        let mut state = FilterState::new();
        assert_eq!(state.build_sort(), None);
        // No schema: nothing to choose from
        state.next_sort_field();
        assert_eq!(state.build_sort(), None);

        state.set_fields(vec!["age".into(), "name".into()]);
        state.next_sort_field();
        assert_eq!(state.build_sort(), Some(json!({"age": 1})));
        state.toggle_sort_direction();
        assert_eq!(state.build_sort(), Some(json!({"age": -1})));
        state.next_sort_field();
        assert_eq!(state.build_sort(), Some(json!({"name": -1})));
        // After the last field: "no sort", the direction is kept
        state.next_sort_field();
        assert_eq!(state.build_sort(), None);
        assert_eq!(state.sort_direction, SortDirection::Desc);
        state.prev_sort_field();
        assert_eq!(state.build_sort(), Some(json!({"name": -1})));
    }

    #[test]
    fn test_add_filter_takes_inputs_and_resets_the_form() {
        let mut state = FilterState::new();
        // Empty field: nothing is added
        state.value_input.set_value("x");
        state.add_filter();
        assert!(state.filters.is_empty());
        assert_eq!(state.value_input.value(), "x");

        state.field_input.set_value("age");
        state.value_input.set_value("30");
        state.next_operator();
        state.next_operator();
        state.next_operator();
        state.next_operator();
        assert_eq!(state.operator, FilterOperator::GreaterThan);
        state.focus = FilterFocus::Value;

        state.add_filter();

        assert_eq!(state.filters.len(), 1);
        assert_eq!(state.build_query(), json!({"age": {"$gt": 30}}));
        assert_eq!(state.field_input.value(), "");
        assert_eq!(state.value_input.value(), "");
        assert_eq!(
            (state.field_input.cursor(), state.value_input.cursor()),
            (0, 0)
        );
        assert_eq!(state.operator, FilterOperator::Equals);
        assert_eq!(state.operator_idx, 0);
        assert_eq!(state.focus, FilterFocus::Field);
    }

    /// The next condition starts from the full suggestion list, not the
    /// previous field's matches
    #[test]
    fn test_add_filter_refreshes_the_suggestions() {
        let mut state = FilterState::new();
        state.set_fields(vec!["name".into(), "Age".into(), "address.city".into()]);
        type_into(&mut state, "aG");
        assert_eq!(state.filtered_suggestions, vec!["Age"]);
        state.focus = FilterFocus::Value;
        type_into(&mut state, "30");
        state.add_filter();
        assert_eq!(state.filtered_suggestions.len(), 3);
        state.apply_suggestion();
        assert_eq!(state.field_input.value(), "name");
    }

    #[test]
    fn test_operator_cycle_follows_the_displayed_order() {
        let mut state = FilterState::new();
        state.prev_operator();
        assert_eq!(state.operator, FilterOperator::In);
        state.next_operator();
        assert_eq!(state.operator, FilterOperator::Equals);
        let mut seen = Vec::new();
        for _ in 0..FilterOperator::all().len() {
            seen.push(state.operator);
            state.next_operator();
        }
        assert_eq!(seen, FilterOperator::all());
        assert_eq!(state.operator, FilterOperator::Equals);
    }

    #[test]
    fn test_remove_selected_filter() {
        let mut state = FilterState::new();
        // Nothing to remove
        state.remove_selected_filter();
        for field in ["a", "b", "c"] {
            state
                .filters
                .push(condition(field, FilterOperator::Equals, "1"));
        }
        state.selected_filter = 2;
        state.focus = FilterFocus::Filters;

        state.remove_selected_filter();
        assert_eq!(state.filters.len(), 2);
        assert_eq!(state.selected_filter, 1);
        assert_eq!(state.focus, FilterFocus::Filters);

        state.selected_filter = 0;
        state.remove_selected_filter();
        assert_eq!(state.filters[0].field, "b");
        assert_eq!(state.selected_filter, 0);

        // Last one removed: focus returns to the field input
        state.remove_selected_filter();
        assert!(state.filters.is_empty());
        assert_eq!(state.focus, FilterFocus::Field);
    }

    /// Editing moves the condition out of the list into the form
    #[test]
    fn test_edit_selected_filter_loads_it_into_the_form() {
        let mut state = FilterState::new();
        state
            .filters
            .push(condition("a", FilterOperator::Equals, "1"));
        state
            .filters
            .push(condition("város", FilterOperator::StartsWith, "Győ"));
        state.selected_filter = 1;
        state.focus = FilterFocus::Filters;

        state.edit_selected_filter();

        assert_eq!(state.filters.len(), 1);
        assert_eq!(state.selected_filter, 0);
        assert_eq!(state.field_input.value(), "város");
        assert_eq!(state.value_input.value(), "Győ");
        // Cursors are character positions
        assert_eq!(
            (state.field_input.cursor(), state.value_input.cursor()),
            (5, 3)
        );
        assert_eq!(state.operator, FilterOperator::StartsWith);
        assert_eq!(FilterOperator::all()[state.operator_idx], state.operator);
        assert_eq!(state.focus, FilterFocus::Field);

        // Adding it back puts it at the end
        state.add_filter();
        assert_eq!(state.filters[1].field, "város");
    }

    #[test]
    fn test_move_filter_up_and_down() {
        let mut state = FilterState::new();
        for field in ["a", "b", "c"] {
            state
                .filters
                .push(condition(field, FilterOperator::Equals, "1"));
        }
        let order = |s: &FilterState| -> Vec<String> {
            s.filters.iter().map(|f| f.field.clone()).collect()
        };

        state.move_filter_up();
        assert_eq!(order(&state), ["a", "b", "c"]);
        state.move_filter_down();
        assert_eq!(order(&state), ["b", "a", "c"]);
        assert_eq!(state.selected_filter, 1);
        state.move_filter_down();
        state.move_filter_down();
        assert_eq!(order(&state), ["b", "c", "a"]);
        assert_eq!(state.selected_filter, 2);
        state.move_filter_up();
        assert_eq!(order(&state), ["b", "a", "c"]);
    }

    #[test]
    fn test_input_editing_with_multibyte() {
        let mut state = FilterState::new();
        type_into(&mut state, "név");
        assert_eq!(
            (state.field_input.value(), state.field_input.cursor()),
            ("név", 3)
        );
        key(&mut state, KeyCode::Left);
        key(&mut state, KeyCode::Backspace);
        assert_eq!(
            (state.field_input.value(), state.field_input.cursor()),
            ("nv", 1)
        );
        key(&mut state, KeyCode::Home);
        key(&mut state, KeyCode::Backspace);
        assert_eq!(state.field_input.value(), "nv");
        key(&mut state, KeyCode::Char('ű'));
        assert_eq!(state.field_input.value(), "űnv");
        // Delete removes the character under the cursor
        key(&mut state, KeyCode::Delete);
        assert_eq!(state.field_input.value(), "űv");
        key(&mut state, KeyCode::End);
        key(&mut state, KeyCode::Right);
        assert_eq!(state.field_input.cursor(), 2);

        // Operator focus: typing goes nowhere
        state.focus = FilterFocus::Operator;
        assert_eq!(key(&mut state, KeyCode::Char('x')), InputOutcome::Ignored);
        assert_eq!(key(&mut state, KeyCode::Backspace), InputOutcome::Ignored);
        assert_eq!(state.field_input.value(), "űv");

        state.focus = FilterFocus::Value;
        type_into(&mut state, "Győr");
        key(&mut state, KeyCode::Left);
        key(&mut state, KeyCode::Left);
        key(&mut state, KeyCode::Backspace);
        assert_eq!(
            (state.value_input.value(), state.value_input.cursor()),
            ("Gőr", 1)
        );
        // Ctrl+U clears to the start
        state.handle_field_input(Input::Key(KeyCode::Char('u'), KeyModifiers::CONTROL));
        assert_eq!(
            (state.value_input.value(), state.value_input.cursor()),
            ("őr", 0)
        );
    }

    #[test]
    fn test_suggestions_follow_the_field_input() {
        let mut state = FilterState::new();
        state.set_fields(vec!["name".into(), "Age".into(), "address.city".into()]);
        assert!(state.show_suggestions);
        assert_eq!(state.filtered_suggestions.len(), 3);

        // Case-insensitive substring match
        type_into(&mut state, "aG");
        assert_eq!(state.filtered_suggestions, vec!["Age"]);
        state.apply_suggestion();
        assert_eq!(
            (state.field_input.value(), state.field_input.cursor()),
            ("Age", 3)
        );
        assert!(!state.show_suggestions);

        // No match: the list is hidden
        key(&mut state, KeyCode::Char('z'));
        assert!(state.filtered_suggestions.is_empty());
        assert!(!state.show_suggestions);

        // Any edit refreshes them, not only typing at the end
        state.handle_field_input(Input::Key(KeyCode::Char('u'), KeyModifiers::CONTROL));
        assert_eq!(state.filtered_suggestions.len(), 3);

        // Selection wraps
        state.update_suggestions();
        state.suggestion_up();
        assert_eq!(state.suggestion_idx, 2);
        state.suggestion_down();
        assert_eq!(state.suggestion_idx, 0);
    }

    #[test]
    fn test_focus_cycle_skips_the_empty_filter_list() {
        let mut state = FilterState::new();
        let mut seen = Vec::new();
        for _ in 0..4 {
            state.next_focus();
            seen.push(state.focus);
        }
        assert_eq!(
            seen,
            vec![
                FilterFocus::Operator,
                FilterFocus::Value,
                FilterFocus::SortField,
                FilterFocus::Field
            ]
        );
        state.prev_focus();
        assert_eq!(state.focus, FilterFocus::SortField);
        state.prev_focus();
        assert_eq!(state.focus, FilterFocus::Value);

        state
            .filters
            .push(condition("a", FilterOperator::Equals, "1"));
        state.next_focus();
        assert_eq!(state.focus, FilterFocus::Filters);
        state.next_focus();
        assert_eq!(state.focus, FilterFocus::SortField);
        state.prev_focus();
        assert_eq!(state.focus, FilterFocus::Filters);
    }

    /// Reopening the modal keeps the conditions and the sort
    #[test]
    fn test_reset_inputs_keeps_filters_and_sort() {
        let mut state = FilterState::new();
        state.set_fields(vec!["a".into()]);
        state
            .filters
            .push(condition("a", FilterOperator::Equals, "1"));
        state.next_sort_field();
        state.field_input.set_value("x");
        state.value_input.set_value("y");
        state.next_operator();
        state.focus = FilterFocus::Value;
        state.error = Some("hiba".to_string());

        state.reset_inputs();

        assert_eq!(state.field_input.value(), "");
        assert_eq!(state.value_input.value(), "");
        assert_eq!(state.operator, FilterOperator::Equals);
        assert_eq!(state.focus, FilterFocus::Field);
        assert_eq!(state.error, None);
        assert_eq!(state.filters.len(), 1);
        assert_eq!(state.sort_field.as_deref(), Some("a"));
    }

    /// The schema is reloaded on every open; a sort index kept from a wider
    /// schema used to index `all_fields` out of bounds
    #[test]
    fn test_stale_sort_field_idx_is_clamped() {
        let mut state = FilterState::new();
        state.set_fields(vec!["a".into(), "b".into(), "c".into(), "d".into()]);
        state.prev_sort_field();
        assert_eq!(state.sort_field.as_deref(), Some("d"));
        assert_eq!(state.sort_field_idx, 4);

        state.set_fields(vec!["a".into()]);
        state.prev_sort_field();
        assert_eq!(state.sort_field, None);

        // Stale index 4 counts as the last option (1): next wraps to "no sort"
        state.sort_field_idx = 4;
        state.next_sort_field();
        assert_eq!(state.sort_field, None);
        state.next_sort_field();
        assert_eq!(state.sort_field.as_deref(), Some("a"));
    }
}
