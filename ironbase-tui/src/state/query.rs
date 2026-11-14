//! Query builder state

use serde_json::Value;

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
    /// Query explain result (plan details)
    pub explain_result: Option<Value>,
    /// Show explain panel instead of results
    pub show_explain: bool,
}

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
            explain_result: None,
            show_explain: false,
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
            explain_result: None,
            show_explain: false,
        }
    }

    /// Check if we have results to navigate
    pub fn has_results(&self) -> bool {
        self.results
            .as_ref()
            .map(|r| !r.is_empty())
            .unwrap_or(false)
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

    /// Clear results (when user edits query)
    pub fn clear_results(&mut self) {
        self.results = None;
        self.result_index = 0;
        self.total_count = 0;
        self.explain_result = None;
        self.show_explain = false;
    }

    /// Toggle between results and explain view
    pub fn toggle_explain_view(&mut self) {
        if self.explain_result.is_some() || self.results.is_some() {
            self.show_explain = !self.show_explain;
        }
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn query(lines: &[&str], line: usize, col: usize) -> QueryState {
        QueryState {
            lines: lines.iter().map(|l| l.to_string()).collect(),
            cursor_line: line,
            cursor_col: col,
            ..QueryState::default()
        }
    }

    #[test]
    fn test_new_query_is_an_empty_object_with_cursor_inside() {
        let state = QueryState::new("users".to_string());
        assert_eq!(state.lines, vec!["{", "  ", "}"]);
        assert_eq!((state.cursor_line, state.cursor_col), (1, 2));
        assert_eq!(state.parse_query(), Ok(json!({})));
    }

    #[test]
    fn test_insert_is_by_character_not_byte() {
        let mut state = query(&["{\"név\": \"\"}"], 0, 9);
        state.insert_char('É');
        state.insert_char('v');
        state.insert_char('a');
        assert_eq!(state.lines, vec!["{\"név\": \"Éva\"}"]);
        assert_eq!(state.cursor_col, 12);
        assert_eq!(state.parse_query(), Ok(json!({"név": "Éva"})));
    }

    #[test]
    fn test_backspace_with_multibyte_and_at_line_start() {
        let mut state = query(&["őá", "űé"], 1, 1);
        state.backspace();
        assert_eq!(state.lines, vec!["őá", "é"]);
        assert_eq!((state.cursor_line, state.cursor_col), (1, 0));

        // At line start: joins with the previous line, cursor at the seam
        state.backspace();
        assert_eq!(state.lines, vec!["őáé"]);
        assert_eq!((state.cursor_line, state.cursor_col), (0, 2));

        // First line, first column: nothing
        state.cursor_col = 0;
        state.backspace();
        assert_eq!(state.lines, vec!["őáé"]);
    }

    #[test]
    fn test_newline_and_tab() {
        let mut state = query(&["{űő}"], 0, 2);
        state.insert_newline();
        assert_eq!(state.lines, vec!["{ű", "ő}"]);
        assert_eq!((state.cursor_line, state.cursor_col), (1, 0));
        state.insert_tab();
        assert_eq!(state.lines, vec!["{ű", "  ő}"]);
        assert_eq!(state.cursor_col, 2);
    }

    #[test]
    fn test_cursor_movement_wraps_and_clamps() {
        let mut state = query(&["hosszú", "ű", ""], 0, 6);
        state.cursor_right();
        assert_eq!((state.cursor_line, state.cursor_col), (1, 0));
        state.cursor_left();
        assert_eq!((state.cursor_line, state.cursor_col), (0, 6));
        state.cursor_down();
        assert_eq!((state.cursor_line, state.cursor_col), (1, 1));
        state.cursor_down();
        assert_eq!((state.cursor_line, state.cursor_col), (2, 0));
        state.cursor_down();
        state.cursor_right();
        assert_eq!((state.cursor_line, state.cursor_col), (2, 0));
        state.cursor_up();
        state.cursor_up();
        state.cursor_up();
        state.cursor_left();
        assert_eq!((state.cursor_line, state.cursor_col), (0, 0));
    }

    /// Editing drops the results of the previous run and the error
    #[test]
    fn test_editing_clears_results_and_error() {
        let mut state = QueryState::new("users".to_string());
        state.results = Some(vec![json!({"_id": 1}), json!({"_id": 2})]);
        state.total_count = 2;
        state.explain_result = Some(json!({"plan": "scan"}));
        state.show_explain = true;
        state.error = Some("hiba".to_string());
        state.result_down();
        assert_eq!(state.result_index, 1);
        state.result_down();
        assert_eq!(state.result_index, 1);

        state.insert_char('x');

        assert_eq!(state.results, None);
        assert_eq!((state.result_index, state.total_count), (0, 0));
        assert_eq!(state.explain_result, None);
        assert!(!state.show_explain);
        assert_eq!(state.error, None);
        assert!(!state.has_results());
    }

    #[test]
    fn test_templates() {
        let mut state = QueryState::new("users".to_string());
        state.toggle_templates();
        assert!(state.show_templates);
        state.template_up();
        assert_eq!(state.template_index, 0);
        for _ in 0..QUERY_TEMPLATES.len() + 3 {
            state.template_down();
        }
        assert_eq!(state.template_index, QUERY_TEMPLATES.len() - 1);
        state.template_index = 3;
        state.apply_template();
        assert!(!state.show_templates);
        assert_eq!(state.lines, vec!["{\"field\": {\"$gt\": 0}}"]);
        assert_eq!((state.cursor_line, state.cursor_col), (0, 21));
        assert_eq!(state.parse_query(), Ok(json!({"field": {"$gt": 0}})));

        // Every template is valid JSON
        for (index, (name, _)) in QUERY_TEMPLATES.iter().enumerate() {
            state.template_index = index;
            state.apply_template();
            assert!(state.parse_query().is_ok(), "{}", name);
        }
    }

    #[test]
    fn test_explain_view_toggles_only_with_something_to_show() {
        let mut state = QueryState::new("users".to_string());
        state.toggle_explain_view();
        assert!(!state.show_explain);
        state.results = Some(vec![]);
        state.toggle_explain_view();
        assert!(state.show_explain);
        state.toggle_explain_view();
        assert!(!state.show_explain);
    }

    #[test]
    fn test_invalid_query_is_an_error() {
        let state = query(&["{\"a\": }"], 0, 0);
        assert!(state.parse_query().is_err());
    }
}
