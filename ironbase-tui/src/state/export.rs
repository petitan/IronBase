//! Export state

use crate::modals::export::ExportFormat;

/// Export state
#[derive(Debug, Clone)]
pub struct ExportState {
    pub collection: String,
    pub doc_count: usize,
    /// `doc_count` is the filtered count (an active filter is exported)
    pub filtered: bool,
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
            filtered: false,
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
            filtered: false,
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

    fn update_extension(&mut self) {
        // Replace the extension of the final path component only
        // (a dot in a directory name is not an extension)
        let path = std::path::Path::new(&self.file_path).with_extension(self.format.extension());
        self.file_path = path.to_string_lossy().into_owned();
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

#[cfg(test)]
mod tests {
    use super::*;

    fn path_after_toggle(path: &str) -> String {
        let mut state = ExportState::new("users".to_string(), 0);
        state.file_path = path.to_string();
        state.toggle_format();
        state.file_path
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
