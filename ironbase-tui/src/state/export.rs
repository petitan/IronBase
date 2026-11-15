//! Export state

use crate::modals::export::ExportFormat;
use crate::widgets::{InputOutcome, TextInput};
use crossterm::event::{KeyCode, KeyModifiers};

/// Export state
#[derive(Debug, Clone)]
pub struct ExportState {
    pub collection: String,
    pub doc_count: usize,
    /// `doc_count` is the filtered count (an active filter is exported)
    pub filtered: bool,
    pub format: ExportFormat,
    pub file_path: TextInput,
    pub error: Option<String>,
    pub message: Option<String>,
}

impl Default for ExportState {
    fn default() -> Self {
        Self {
            collection: String::new(),
            doc_count: 0,
            filtered: false,
            format: ExportFormat::Json,
            file_path: TextInput::default(),
            error: None,
            message: None,
        }
    }
}

impl ExportState {
    pub fn new(collection: String, doc_count: usize) -> Self {
        let file_path = TextInput::new(format!("{}.json", collection));
        Self {
            collection,
            doc_count,
            filtered: false,
            format: ExportFormat::Json,
            file_path,
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

    fn update_extension(&mut self) {
        // Replace the extension of the final path component only
        // (a dot in a directory name is not an extension)
        let path =
            std::path::Path::new(self.file_path.value()).with_extension(self.format.extension());
        self.file_path
            .set_value(path.to_string_lossy().into_owned());
    }

    /// An editing key for the path; a change clears the error
    pub fn handle_input(&mut self, code: KeyCode, modifiers: KeyModifiers) -> InputOutcome {
        let outcome = self.file_path.handle_key(code, modifiers);
        if outcome == InputOutcome::Changed {
            self.error = None;
        }
        outcome
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn path_after_toggle(path: &str) -> String {
        let mut state = ExportState::new("users".to_string(), 0);
        state.file_path.set_value(path);
        state.toggle_format();
        state.file_path.value().to_string()
    }

    #[test]
    fn test_update_extension_only_touches_final_component() {
        assert_eq!(path_after_toggle("users.json"), "users.csv");
        assert_eq!(path_after_toggle("./out/users"), "./out/users.csv");
        assert_eq!(
            path_after_toggle("/data/v1.2/users"),
            "/data/v1.2/users.csv"
        );
        assert_eq!(
            path_after_toggle("/data/v1.2/users.json"),
            "/data/v1.2/users.csv"
        );
        assert_eq!(path_after_toggle("../users"), "../users.csv");
    }
}
