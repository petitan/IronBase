//! Query builder state

use crate::widgets::{Input, InputOutcome, TextArea};
use ironbase_mcp_types::responses::QueryPlanSummary;
use serde_json::Value;

/// A query template (its name comes from the language layer)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueryTemplate {
    All,
    ById,
    Equals,
    GreaterThan,
    LessThan,
    In,
    Exists,
    Regex,
    And,
    Or,
}

impl QueryTemplate {
    pub fn name(self) -> &'static str {
        crate::i18n::t().query_template(self)
    }
}

/// Query templates
pub const QUERY_TEMPLATES: &[(QueryTemplate, &str)] = &[
    (QueryTemplate::All, "{}"),
    (QueryTemplate::ById, "{\"_id\": \"\"}"),
    (QueryTemplate::Equals, "{\"field\": \"value\"}"),
    (QueryTemplate::GreaterThan, "{\"field\": {\"$gt\": 0}}"),
    (QueryTemplate::LessThan, "{\"field\": {\"$lt\": 100}}"),
    (QueryTemplate::In, "{\"field\": {\"$in\": [\"a\", \"b\"]}}"),
    (QueryTemplate::Exists, "{\"field\": {\"$exists\": true}}"),
    (
        QueryTemplate::Regex,
        "{\"field\": {\"$regex\": \"pattern\"}}",
    ),
    (QueryTemplate::And, "{\"$and\": [{\"a\": 1}, {\"b\": 2}]}"),
    (QueryTemplate::Or, "{\"$or\": [{\"a\": 1}, {\"b\": 2}]}"),
];

/// Query builder state
#[derive(Debug, Clone)]
pub struct QueryState {
    pub collection: String,
    pub editor: TextArea,
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
    pub explain_result: Option<QueryPlanSummary>,
    /// Show explain panel instead of results
    pub show_explain: bool,
}

impl Default for QueryState {
    fn default() -> Self {
        Self {
            collection: String::new(),
            editor: TextArea::with_cursor("{}", 0, 1),
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
            editor: TextArea::with_cursor("{\n  \n}", 1, 2),
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
            self.editor = TextArea::with_cursor(template, 0, usize::MAX);
            self.show_templates = false;
            self.error = None;
        }
    }

    /// Edit the query; an edit drops the previous run's results and the error
    pub fn handle_input(&mut self, input: Input) -> InputOutcome {
        // The template list is over the editor
        if self.show_templates {
            return InputOutcome::Ignored;
        }
        let outcome = self.editor.apply(input);
        if outcome == InputOutcome::Changed {
            self.clear_results();
            self.error = None;
        }
        outcome
    }

    pub fn get_query_text(&self) -> String {
        self.editor.text()
    }

    pub fn parse_query(&self) -> Result<Value, String> {
        let text = self.get_query_text();
        serde_json::from_str(&text).map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyModifiers};
    use serde_json::json;

    #[test]
    fn test_new_query_is_an_empty_object_with_cursor_inside() {
        let state = QueryState::new("users".to_string());
        assert_eq!(state.editor.text(), "{\n  \n}");
        assert_eq!(state.editor.cursor(), (1, 2));
        assert_eq!(state.parse_query(), Ok(json!({})));
    }

    #[test]
    fn test_typed_query_parses() {
        let mut state = QueryState::new("users".to_string());
        for c in "\"név\": \"Éva\"".chars() {
            state.handle_input(Input::Key(KeyCode::Char(c), KeyModifiers::NONE));
        }
        assert_eq!(state.parse_query(), Ok(json!({"név": "Éva"})));
    }

    /// Editing drops the results of the previous run and the error
    #[test]
    fn test_editing_clears_results_and_error() {
        let mut state = QueryState::new("users".to_string());
        state.results = Some(vec![json!({"_id": 1}), json!({"_id": 2})]);
        state.total_count = 2;
        state.explain_result = Some(
            serde_json::from_value(json!({"chosenPlan": {"queryPlan": "CollectionScan"}})).unwrap(),
        );
        state.show_explain = true;
        state.error = Some("hiba".to_string());
        state.result_down();
        assert_eq!(state.result_index, 1);
        state.result_down();
        assert_eq!(state.result_index, 1);

        // A cursor move keeps them
        state.handle_input(Input::Key(KeyCode::Left, KeyModifiers::NONE));
        assert!(state.has_results());
        assert_eq!(
            state.handle_input(Input::Key(KeyCode::Char('x'), KeyModifiers::NONE)),
            InputOutcome::Changed
        );

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
        assert_eq!(state.editor.text(), "{\"field\": {\"$gt\": 0}}");
        assert_eq!(state.editor.cursor(), (0, 21));
        assert_eq!(state.parse_query(), Ok(json!({"field": {"$gt": 0}})));

        // Every template is valid JSON
        for (index, (name, _)) in QUERY_TEMPLATES.iter().enumerate() {
            state.template_index = index;
            state.apply_template();
            assert!(state.parse_query().is_ok(), "{:?}", name);
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
        let state = QueryState {
            editor: TextArea::new("{\"a\": }"),
            ..QueryState::default()
        };
        assert!(state.parse_query().is_err());
    }
}
