//! Export state

use crate::modals::export::ExportFormat;

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
