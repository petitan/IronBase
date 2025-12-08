//! Application state and navigation - Pane-based architecture

use crate::config::Config;
use crate::db::{CollectionInfo, DbWrapper};
use crate::modals::confirm::{ConfirmAction, ConfirmOption, ConfirmState};
use crate::modals::export::ExportFormat;
use crate::theme::{Theme, ThemeName};
use serde_json::Value;
use std::path::PathBuf;

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
    ErrorDetail,
}

/// Search mode enum
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SearchMode {
    #[default]
    Collections,
    Documents,
}

/// Search result item
#[derive(Debug, Clone)]
pub enum SearchResult {
    Collection {
        name: String,
        doc_count: usize,
        match_reason: String,
    },
    Document {
        collection: String,
        doc_id: String,
        preview: String,
        match_field: String,
    },
}

/// Search state
#[derive(Debug, Default)]
pub struct SearchState {
    pub query_input: String,
    pub cursor_pos: usize,
    pub input_active: bool,
    pub mode: SearchMode,
    pub results: Vec<SearchResult>,
    pub selected_result: usize,
    pub result_offset: usize,
    pub last_search: String,
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
    }

    pub fn toggle_mode(&mut self) {
        self.mode = match self.mode {
            SearchMode::Collections => SearchMode::Documents,
            SearchMode::Documents => SearchMode::Collections,
        };
        self.results.clear();
        self.selected_result = 0;
        self.last_search.clear();
    }

    pub fn insert_char(&mut self, c: char) {
        self.query_input.insert(self.cursor_pos, c);
        self.cursor_pos += 1;
    }

    pub fn delete_char(&mut self) {
        if self.cursor_pos > 0 {
            self.cursor_pos -= 1;
            self.query_input.remove(self.cursor_pos);
        }
    }

    pub fn cursor_left(&mut self) {
        if self.cursor_pos > 0 {
            self.cursor_pos -= 1;
        }
    }

    pub fn cursor_right(&mut self) {
        if self.cursor_pos < self.query_input.len() {
            self.cursor_pos += 1;
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
            if self.cursor_col <= line.len() {
                line.insert(self.cursor_col, c);
                self.cursor_col += 1;
            }
        }
        self.error = None;
    }

    /// Delete character before cursor (backspace)
    pub fn backspace(&mut self) {
        if self.cursor_col > 0 {
            if let Some(line) = self.lines.get_mut(self.cursor_line) {
                if self.cursor_col <= line.len() {
                    line.remove(self.cursor_col - 1);
                    self.cursor_col -= 1;
                }
            }
        } else if self.cursor_line > 0 {
            // Join with previous line
            let current_line = self.lines.remove(self.cursor_line);
            self.cursor_line -= 1;
            if let Some(prev_line) = self.lines.get_mut(self.cursor_line) {
                self.cursor_col = prev_line.len();
                prev_line.push_str(&current_line);
            }
        }
        self.error = None;
    }

    /// Insert a new line at cursor
    pub fn insert_newline(&mut self) {
        if let Some(line) = self.lines.get_mut(self.cursor_line) {
            let rest = line.split_off(self.cursor_col);
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
            self.cursor_col = self
                .lines
                .get(self.cursor_line)
                .map(|l| l.len())
                .unwrap_or(0);
        }
    }

    /// Move cursor right
    pub fn cursor_right(&mut self) {
        let line_len = self
            .lines
            .get(self.cursor_line)
            .map(|l| l.len())
            .unwrap_or(0);
        if self.cursor_col < line_len {
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
            let line_len = self
                .lines
                .get(self.cursor_line)
                .map(|l| l.len())
                .unwrap_or(0);
            self.cursor_col = self.cursor_col.min(line_len);
        }
    }

    /// Move cursor down
    pub fn cursor_down(&mut self) {
        if self.cursor_line + 1 < self.lines.len() {
            self.cursor_line += 1;
            let line_len = self
                .lines
                .get(self.cursor_line)
                .map(|l| l.len())
                .unwrap_or(0);
            self.cursor_col = self.cursor_col.min(line_len);
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
        }
    }

    pub fn insert_char(&mut self, c: char) {
        if let Some(line) = self.lines.get_mut(self.cursor_line) {
            if self.cursor_col <= line.len() {
                line.insert(self.cursor_col, c);
                self.cursor_col += 1;
            }
        }
        self.error = None;
    }

    pub fn backspace(&mut self) {
        if self.cursor_col > 0 {
            if let Some(line) = self.lines.get_mut(self.cursor_line) {
                if self.cursor_col <= line.len() {
                    line.remove(self.cursor_col - 1);
                    self.cursor_col -= 1;
                }
            }
        } else if self.cursor_line > 0 {
            let current_line = self.lines.remove(self.cursor_line);
            self.cursor_line -= 1;
            if let Some(prev_line) = self.lines.get_mut(self.cursor_line) {
                self.cursor_col = prev_line.len();
                prev_line.push_str(&current_line);
            }
        }
        self.error = None;
    }

    pub fn insert_newline(&mut self) {
        if let Some(line) = self.lines.get_mut(self.cursor_line) {
            let rest = line.split_off(self.cursor_col);
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
            self.cursor_col = self
                .lines
                .get(self.cursor_line)
                .map(|l| l.len())
                .unwrap_or(0);
        }
    }

    pub fn cursor_right(&mut self) {
        let line_len = self
            .lines
            .get(self.cursor_line)
            .map(|l| l.len())
            .unwrap_or(0);
        if self.cursor_col < line_len {
            self.cursor_col += 1;
        } else if self.cursor_line + 1 < self.lines.len() {
            self.cursor_line += 1;
            self.cursor_col = 0;
        }
    }

    pub fn cursor_up(&mut self) {
        if self.cursor_line > 0 {
            self.cursor_line -= 1;
            let line_len = self
                .lines
                .get(self.cursor_line)
                .map(|l| l.len())
                .unwrap_or(0);
            self.cursor_col = self.cursor_col.min(line_len);
        }
    }

    pub fn cursor_down(&mut self) {
        if self.cursor_line + 1 < self.lines.len() {
            self.cursor_line += 1;
            let line_len = self
                .lines
                .get(self.cursor_line)
                .map(|l| l.len())
                .unwrap_or(0);
            self.cursor_col = self.cursor_col.min(line_len);
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

/// Main application state
pub struct App {
    // Quit flag
    pub should_quit: bool,

    // Focus management
    pub active_pane: Pane,
    pub modal: Option<Modal>,

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

    // Export state
    pub export_state: ExportState,

    // Config
    pub config: Config,

    // Status
    pub status_message: Option<String>,
    pub error_message: Option<String>,
    pub error_scroll: usize,

    // Loading state
    pub loading: Option<String>,
    pub loading_frame: usize,

    // Startup phase (splash screen)
    pub startup: bool,
}

impl App {
    pub fn new(config: Config) -> Self {
        let theme_name = config.theme;
        let theme = Theme::from_name(theme_name);

        Self {
            should_quit: false,
            active_pane: Pane::Collections,
            modal: None,
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
            detail_scroll: 0,
            search: SearchState::new(),
            confirm: ConfirmState::default(),
            insert: InsertState::default(),
            index_state: IndexState::default(),
            query_state: QueryState::default(),
            export_state: ExportState::default(),
            config,
            status_message: None,
            error_message: None,
            error_scroll: 0,
            loading: None,
            loading_frame: 0,
            startup: true,
        }
    }

    // === Loading state ===

    /// Set loading state with message
    pub fn set_loading(&mut self, msg: impl Into<String>) {
        self.loading = Some(msg.into());
    }

    /// Clear loading state
    pub fn clear_loading(&mut self) {
        self.loading = None;
    }

    /// Check if loading
    pub fn is_loading(&self) -> bool {
        self.loading.is_some()
    }

    /// Get loading spinner character (animated)
    pub fn loading_spinner(&self) -> char {
        const SPINNERS: [char; 8] = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧'];
        SPINNERS[self.loading_frame % SPINNERS.len()]
    }

    /// Advance loading animation frame
    pub fn tick_loading(&mut self) {
        self.loading_frame = self.loading_frame.wrapping_add(1);
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

    /// Open search modal
    pub fn open_search(&mut self) {
        self.modal = Some(Modal::Search);
        self.search.input_active = true;
        self.search.query_input.clear();
        self.search.cursor_pos = 0;
        self.search.results.clear();
        self.search.selected_result = 0;
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

        self.insert = InsertState::new(coll_name);
        self.modal = Some(Modal::Insert);
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

        // Indexes will be loaded async later
        self.index_state = IndexState::new(coll_name, vec![]);
        self.modal = Some(Modal::Index);
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

    /// Refresh collections list (async)
    pub async fn refresh_collections_async(&mut self) -> anyhow::Result<()> {
        self.set_loading("Kollekciok betoltese...");
        let result = self.refresh_collections_inner().await;
        self.clear_loading();
        result
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
        self.set_loading(format!("Dokumentumok betoltese: {}...", coll_name));
        let result = self.refresh_documents_inner().await;
        self.clear_loading();
        result
    }

    async fn refresh_documents_inner(&mut self) -> anyhow::Result<()> {
        let (coll_name, needs_details) = match self.collections.get(self.selected_collection) {
            Some(coll) => (coll.name.clone(), !coll.is_loaded()),
            None => return Ok(()),
        };

        if let Some(db) = &self.db {
            let skip = self.doc_scroll_offset;
            let docs = db.get_documents(&coll_name, skip, self.page_size).await?;
            self.documents = docs;

            // Lazy load: only fetch details if not already loaded
            if needs_details {
                let (doc_count, index_names) = db.load_collection_details(&coll_name).await?;
                self.total_docs = doc_count;

                // Update cached collection info
                if let Some(coll) = self.collections.get_mut(self.selected_collection) {
                    coll.doc_count = Some(doc_count);
                    coll.index_count = Some(index_names.len());
                }

                // Update index_state for UI display
                self.index_state.indexes = index_names;
            } else {
                // Use cached count
                if let Some(coll) = self.collections.get(self.selected_collection) {
                    self.total_docs = coll.doc_count.unwrap_or(0);
                }
            }
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
                if self.detail_scroll < u16::MAX as usize {
                    self.detail_scroll += 1;
                }
            }
        }
    }

    /// Page up (async)
    pub async fn page_up_async(&mut self) {
        match self.active_pane {
            Pane::Collections => {
                self.selected_collection = self.selected_collection.saturating_sub(10);
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
                self.detail_scroll = self.detail_scroll.saturating_sub(10);
            }
        }
    }

    /// Page down (async)
    pub async fn page_down_async(&mut self) {
        match self.active_pane {
            Pane::Collections => {
                let max = self.collections.len().saturating_sub(1);
                self.selected_collection = (self.selected_collection + 10).min(max);
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
                self.detail_scroll = (self.detail_scroll + 10).min(u16::MAX as usize);
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
                let last = self.collections.len().saturating_sub(1);
                if self.selected_collection != last {
                    self.selected_collection = last;
                    self.doc_scroll_offset = 0;
                    self.selected_document = 0;
                    let _ = self.refresh_documents_async().await;
                }
            }
            Pane::Documents => {
                let last = self.total_docs.saturating_sub(1);
                self.selected_document = last;
                self.doc_scroll_offset = last.saturating_sub(self.page_size - 1);
                let _ = self.refresh_documents_async().await;
            }
            Pane::Detail => {
                self.detail_scroll = 10000;
            }
        }
    }

    // === Async Search ===

    /// Execute search based on current mode and query (async)
    pub async fn execute_search_async(&mut self) {
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

        match self.search.mode {
            SearchMode::Collections => match db.search_collections(query).await {
                Ok(collections) => {
                    self.search.results = collections
                        .into_iter()
                        .map(|c| SearchResult::Collection {
                            match_reason: format!("nev tartalmazza: '{}'", query),
                            name: c.name,
                            doc_count: c.doc_count.unwrap_or(0),
                        })
                        .collect();
                }
                Err(e) => {
                    self.set_error(format!("Kereses hiba: {}", e));
                }
            },
            SearchMode::Documents => match db.search_documents(query, None, 100).await {
                Ok(matches) => {
                    self.search.results = matches
                        .into_iter()
                        .map(|m| SearchResult::Document {
                            collection: m.collection,
                            doc_id: m.doc_id,
                            preview: m.preview,
                            match_field: m.matched_field,
                        })
                        .collect();
                }
                Err(e) => {
                    self.set_error(format!("Kereses hiba: {}", e));
                }
            },
        }

        if self.search.results.is_empty() {
            self.set_status("Nincs talalat");
        } else {
            self.set_status(format!("{} talalat", self.search.results.len()));
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
            match result {
                SearchResult::Collection { name, .. } => {
                    if let Some(idx) = self.collections.iter().position(|c| c.name == name) {
                        self.selected_collection = idx;
                        self.doc_scroll_offset = 0;
                        self.selected_document = 0;
                        let _ = self.refresh_documents_async().await;
                        self.active_pane = Pane::Documents;
                    }
                }
                SearchResult::Document { collection, .. } => {
                    if let Some(idx) = self.collections.iter().position(|c| c.name == collection) {
                        self.selected_collection = idx;
                        self.doc_scroll_offset = 0;
                        self.selected_document = 0;
                        let _ = self.refresh_documents_async().await;
                        self.active_pane = Pane::Documents;
                    }
                }
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
                self.set_status(format!("Kollekció törlés: {} - hamarosan...", name));
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
                if self.index_state.selected_index >= self.index_state.indexes.len()
                    && !self.index_state.indexes.is_empty()
                {
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

        let Some(db) = &self.db else {
            self.set_error("Nincs megnyitva adatbázis");
            return;
        };

        let file = match File::create(&file_path) {
            Ok(f) => f,
            Err(e) => {
                self.export_state.error = Some(format!("Fájl hiba: {}", e));
                return;
            }
        };
        let mut writer = BufWriter::new(file);

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
                    self.export_state.message =
                        Some(format!("Exportálás... {}/{}", exported, total_docs));
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
                    self.export_state.message =
                        Some(format!("Exportálás... {}/{}", exported, total_docs));
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
