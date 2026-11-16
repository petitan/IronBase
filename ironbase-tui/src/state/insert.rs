//! Insert/Edit document state

use super::types::EditorMode;
use crate::widgets::{InputOutcome, TextArea};
use crossterm::event::{KeyCode, KeyModifiers};
use serde_json::Value;

/// Document editor state - multi-line JSON editor for insert/edit
#[derive(Debug, Clone)]
pub struct InsertState {
    pub editor: TextArea,
    pub collection: String,
    pub error: Option<String>,
    pub mode: EditorMode,
    pub original_doc_id: Option<Value>,
    /// Top-level field names of the document as it was opened for editing.
    /// A field missing from the saved text was removed by the user and must
    /// be `$unset`; `$set` alone would leave it in place.
    pub original_fields: Vec<String>,
}

impl Default for InsertState {
    fn default() -> Self {
        Self {
            editor: TextArea::with_cursor("{\n  \n}", 1, 2),
            collection: String::new(),
            error: None,
            mode: EditorMode::Insert,
            original_doc_id: None,
            original_fields: Vec::new(),
        }
    }
}

impl InsertState {
    /// Create insert state with template (from existing document or default)
    pub fn with_template(collection: String, template: Option<Value>) -> Self {
        let text = match template {
            Some(val) => serde_json::to_string_pretty(&val).unwrap_or_else(|_| "{}".to_string()),
            // Default template for empty collection
            None => "{\n  \"name\": \"\",\n  \"value\": \"\"\n}".to_string(),
        };

        Self {
            editor: TextArea::with_cursor(&text, 1, 2),
            collection,
            error: None,
            mode: EditorMode::Insert,
            original_doc_id: None,
            original_fields: Vec::new(),
        }
    }

    /// Create editor state for editing an existing document
    pub fn edit(collection: String, doc: &Value) -> Self {
        let json_str = serde_json::to_string_pretty(doc).unwrap_or_else(|_| "{}".to_string());
        let doc_id = doc.get("_id").cloned();
        let original_fields = doc
            .as_object()
            .map(|m| m.keys().cloned().collect())
            .unwrap_or_default();

        Self {
            editor: TextArea::with_cursor(&json_str, 1, 0),
            collection,
            error: None,
            mode: EditorMode::Edit,
            original_doc_id: doc_id,
            original_fields,
        }
    }

    /// The update for saving `doc` over the document opened for editing:
    /// `$set` of the edited document plus `$unset` of the top-level fields the
    /// user removed (sent only when there are any, so an edit that removes
    /// nothing sends the same request as before).
    pub fn edit_update(&self, doc: &Value) -> Value {
        let removed: serde_json::Map<String, Value> = self
            .original_fields
            .iter()
            .filter(|field| doc.get(field.as_str()).is_none())
            .map(|field| (field.clone(), Value::String(String::new())))
            .collect();
        if removed.is_empty() {
            serde_json::json!({"$set": doc})
        } else {
            serde_json::json!({"$set": doc, "$unset": removed})
        }
    }

    pub fn is_edit_mode(&self) -> bool {
        self.mode == EditorMode::Edit
    }

    /// Edit the text; an edit clears the error
    pub fn handle_input(&mut self, code: KeyCode, modifiers: KeyModifiers) -> InputOutcome {
        let outcome = self.editor.handle_key(code, modifiers);
        if outcome == InputOutcome::Changed {
            self.error = None;
        }
        outcome
    }

    /// Get the full JSON text
    pub fn get_json_text(&self) -> String {
        self.editor.text()
    }

    /// Validate and parse JSON
    pub fn parse_json(&self) -> Result<serde_json::Value, String> {
        let text = self.get_json_text();
        serde_json::from_str(&text).map_err(|e| e.to_string())
    }
}
