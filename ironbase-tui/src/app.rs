//! Application state and navigation - Pane-based architecture
//!
//! State types are defined in the `state` module and re-exported here for convenience.

use crate::config::Config;
use crate::db::{CollectionInfo, DbWrapper};
use crate::modals::confirm::{ConfirmAction, ConfirmOption, ConfirmState};
use crate::modals::export::ExportFormat;
use crate::theme::{Theme, ThemeName};
use serde_json::Value;
use std::path::PathBuf;

// Import and re-export state types
pub use crate::state::{
    // Types
    ConnectionType,
    DatabaseMode,
    // Database
    DatabaseState,
    // Search
    DocSearchMatch,
    EditorMode,
    // Export
    ExportState,
    // Filter
    FilterCondition,
    FilterFocus,
    FilterOperator,
    FilterState,
    // Fulltext
    FulltextState,
    // Index
    IndexState,
    // Insert
    InsertState,
    Modal,
    NewCollectionState,
    Pane,
    // Query
    QueryState,
    // RAG & Embedding
    RagState,
    // Script
    ScriptConfirmAction,
    ScriptFocus,
    ScriptInfo,
    ScriptMode,
    ScriptResult,
    ScriptState,
    ScriptVersion,
    SearchMode,
    SearchResult,
    SearchState,
    SortDirection,
    // Vector
    VectorSearchState,
    QUERY_TEMPLATES,
};

// === UI Layout Constants ===

/// Terminal overhead (header + command bar + borders)
const TERMINAL_OVERHEAD: u16 = 4;

/// Minimum document page size
const MIN_PAGE_SIZE: u16 = 5;

/// Detail pane scroll step (also used for page jump in collections)
const SCROLL_STEP: usize = 10;

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

    // Fulltext search state
    pub fulltext_state: FulltextState,

    // ACL modal state
    pub acl_state: crate::modals::acl::AclState,

    // Listener modal state
    pub listener_state: crate::modals::listener::ListenerState,

    // Vector search state
    pub vector_state: VectorSearchState,

    // RAG & Embedding state
    pub rag_state: RagState,

    // Connection type (for permission checks)
    pub connection_type: ConnectionType,

    // Config
    pub config: Config,

    // Status
    pub status_message: Option<String>,
    pub status_message_time: Option<std::time::Instant>,
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
            fulltext_state: FulltextState::default(),
            acl_state: crate::modals::acl::AclState::new(),
            listener_state: crate::modals::listener::ListenerState::new(),
            vector_state: VectorSearchState::default(),
            rag_state: RagState::default(),
            connection_type: ConnectionType::Unknown,
            config,
            status_message: None,
            status_message_time: None,
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
        let total_pages = self.total_docs.div_ceil(self.page_size);
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
    pub fn update_server_info(
        &mut self,
        db_stats: serde_json::Value,
        tools: Vec<serde_json::Value>,
        prompts: Vec<serde_json::Value>,
    ) {
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
    pub fn update_update_state(
        &mut self,
        latest_version: String,
        download_url: String,
        release_notes: Option<String>,
    ) {
        self.update_state
            .update_from_github(latest_version, download_url, release_notes);
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
                        let description = v
                            .get("description")
                            .and_then(|d| d.as_str())
                            .map(String::from);
                        let version = v.get("version").and_then(|v| v.as_u64()).unwrap_or(1) as u32;
                        let tags = v
                            .get("tags")
                            .and_then(|t| t.as_array())
                            .map(|arr| {
                                arr.iter()
                                    .filter_map(|s| s.as_str().map(String::from))
                                    .collect()
                            })
                            .unwrap_or_default();
                        let execution_count = v
                            .get("execution_count")
                            .and_then(|e| e.as_u64())
                            .unwrap_or(0);
                        let last_run_at = v
                            .get("last_run_at")
                            .and_then(|l| l.as_str())
                            .map(String::from);
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
        let Some(script_info) = self
            .script_state
            .scripts
            .get(self.script_state.selected_script)
        else {
            return;
        };
        let script_name = script_info.name.clone();

        let Some(db) = &self.db else {
            self.script_state.error = Some("Nincs megnyitva adatbázis".to_string());
            return;
        };

        match db.script_get(&script_name).await {
            Ok(script_json) => {
                let code = script_json
                    .get("code")
                    .and_then(|c| c.as_str())
                    .unwrap_or("")
                    .to_string();
                let description = script_json
                    .get("description")
                    .and_then(|d| d.as_str())
                    .map(String::from);
                let version = script_json
                    .get("version")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(1) as u32;
                let tags = script_json
                    .get("tags")
                    .and_then(|t| t.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|s| s.as_str().map(String::from))
                            .collect()
                    })
                    .unwrap_or_default();

                self.script_state
                    .load_script(script_name, code, description, tags, version);
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

        match db
            .script_save(&self.script_state.name, &code, description, tags)
            .await
        {
            Ok(result) => {
                let new_version =
                    result.get("version").and_then(|v| v.as_u64()).unwrap_or(1) as u32;
                self.script_state.version = new_version;
                self.script_state.dirty = false;
                self.script_state.message = Some(format!("Script mentve (v{})", new_version));
                self.script_state.error = None;
                // Reload script list to show new/updated scripts
                self.load_scripts_async().await;
            }
            Err(e) => {
                self.script_state.error = Some(format!("Mentés sikertelen: {}", e));
            }
        }
    }

    /// Delete selected script
    pub async fn delete_script_async(&mut self) {
        let Some(script_info) = self
            .script_state
            .scripts
            .get(self.script_state.selected_script)
        else {
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
                let success = result_json
                    .get("success")
                    .and_then(|s| s.as_bool())
                    .unwrap_or(true);
                let output = result_json
                    .get("result")
                    .cloned()
                    .unwrap_or(serde_json::Value::Null);
                let logs = result_json
                    .get("logs")
                    .and_then(|l| l.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|s| s.as_str().map(String::from))
                            .collect()
                    })
                    .unwrap_or_default();
                let error_msg = result_json
                    .get("error")
                    .and_then(|e| e.as_str())
                    .map(String::from);

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
                        let description = v
                            .get("description")
                            .and_then(|d| d.as_str())
                            .map(String::from);
                        let created_at = v
                            .get("created_at")
                            .and_then(|c| c.as_str())
                            .unwrap_or("")
                            .to_string();
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
        let Some(version_info) = self
            .script_state
            .versions
            .get(self.script_state.selected_version)
        else {
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
                self.script_state.message =
                    Some(format!("Rollback sikeres, új verzió: v{}", new_version));
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
        self.set_status(format!("Tema: {}", self.theme_name.name()));
    }

    // === Status ===

    /// Set status message with auto-clear timestamp
    pub fn set_status(&mut self, msg: impl Into<String>) {
        self.status_message = Some(msg.into());
        self.status_message_time = Some(std::time::Instant::now());
    }

    /// Clear status message if it has been visible long enough
    pub fn clear_status_if_expired(&mut self) {
        if let Some(time) = self.status_message_time {
            if time.elapsed() > std::time::Duration::from_secs(2) {
                self.status_message = None;
                self.status_message_time = None;
            }
        }
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
            // New structure: database.path
            if let Some(database) = stats.get("database") {
                if let Some(path_str) = database.get("path").and_then(|v| v.as_str()) {
                    self.db_path = Some(std::path::PathBuf::from(path_str));
                }
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
        let coll_name = match self.collections.get(self.selected_collection) {
            Some(coll) => coll.name.clone(),
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

            // BUG FIX: Always refresh total_docs, not just when needs_details
            if let Some(filter) = &self.active_filter {
                // When filter is active, count matching documents
                let count = db.count_with_query(&coll_name, filter).await.unwrap_or(0);
                self.total_docs = count;
            } else {
                // No filter - get total collection count
                let (doc_count, index_names) = db.load_collection_details(&coll_name).await?;
                self.total_docs = doc_count;

                // Update cached collection info
                if let Some(coll) = self.collections.get_mut(self.selected_collection) {
                    coll.doc_count = Some(doc_count);
                    coll.index_count = Some(index_names.len());
                }

                // Update index_state for UI display
                self.index_state.indexes = index_names;
            }

            // BUG FIX: Validate selected_document after refresh
            // If document count decreased, selected_document might be out of bounds
            if self.total_docs == 0 {
                self.selected_document = 0;
                self.doc_scroll_offset = 0;
            } else if self.selected_document >= self.total_docs {
                self.selected_document = self.total_docs.saturating_sub(1);
                // Also adjust scroll offset if needed
                if self.doc_scroll_offset > self.selected_document {
                    self.doc_scroll_offset = self.selected_document;
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

    /// Create a new index (async) - supports both single and compound indexes
    pub async fn execute_create_index_async(&mut self) {
        let fields = self.index_state.get_fields();

        if fields.is_empty() {
            self.index_state.error = Some("Mező neve kötelező".to_string());
            return;
        }

        let collection = self.index_state.collection.clone();
        let unique = self.index_state.unique;
        let sparse = self.index_state.sparse;
        let is_compound = self.index_state.is_compound;

        let Some(db) = &self.db else {
            self.set_error("Nincs megnyitva adatbázis");
            return;
        };

        let result = if is_compound && fields.len() > 1 {
            // Compound index
            db.create_compound_index(&collection, &fields, unique, sparse)
                .await
        } else {
            // Single field index
            let field = fields.first().map(|s| s.as_str()).unwrap_or("");
            db.create_index(&collection, field, unique, sparse).await
        };

        match result {
            Ok(()) => {
                let mut flags = Vec::new();
                if unique {
                    flags.push("unique");
                }
                if sparse {
                    flags.push("sparse");
                }
                let flags_str = if flags.is_empty() {
                    String::new()
                } else {
                    format!(" [{}]", flags.join(", "))
                };

                let index_desc = if is_compound && fields.len() > 1 {
                    format!("Compound index: {}{}", fields.join(", "), flags_str)
                } else {
                    format!(
                        "Index: {}{}",
                        fields.first().unwrap_or(&String::new()),
                        flags_str
                    )
                };
                self.index_state.message = Some(format!("{} létrehozva", index_desc));
                self.index_state.cancel_create();
                self.index_state.indexes = self.get_current_indexes_async().await;
                // Refresh collection list to update index_count
                let _ = self.refresh_collections_async().await;
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
                // Refresh collection list to update index_count
                let _ = self.refresh_collections_async().await;
            }
            Err(e) => {
                self.index_state.error = Some(format!("Hiba: {}", e));
            }
        }
    }

    /// Analyze indexes - refresh statistics (async)
    pub async fn execute_analyze_index_async(&mut self) {
        let collection = self.index_state.collection.clone();

        let Some(db) = &self.db else {
            self.set_error("Nincs megnyitva adatbázis");
            return;
        };

        // Set analyzing state
        self.index_state.is_analyzing = true;
        self.index_state.message = Some("Analyzing indexes...".to_string());

        // Refresh statistics
        match db.refresh_index_stats(&collection).await {
            Ok(()) => {
                // Load updated statistics
                match db.get_index_statistics(&collection).await {
                    Ok(stats) => {
                        self.index_state.update_statistics(stats);
                        self.index_state.message = Some("Index statistics frissítve!".to_string());
                    }
                    Err(e) => {
                        self.index_state.is_analyzing = false;
                        self.index_state.error = Some(format!("Stats load error: {}", e));
                    }
                }
            }
            Err(e) => {
                self.index_state.is_analyzing = false;
                self.index_state.error = Some(format!("Analyze error: {}", e));
            }
        }
    }

    /// Load index statistics (async) - call when opening index modal
    pub async fn load_index_statistics_async(&mut self) {
        let collection = self.index_state.collection.clone();

        let Some(db) = &self.db else {
            return;
        };

        if let Ok(stats) = db.get_index_statistics(&collection).await {
            self.index_state.update_statistics(stats);
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

    /// Execute query explain (async) - shows query plan
    pub async fn execute_explain_async(&mut self) {
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

        match db.explain(&collection, &query).await {
            Ok(plan) => {
                self.query_state.explain_result = Some(plan);
                self.query_state.show_explain = true;
                self.query_state.error = None;
                self.set_status("Query plan lekérdezve");
            }
            Err(e) => {
                self.query_state.error = Some(format!("Explain hiba: {}", e));
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
        let _total_docs = self.export_state.doc_count;

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
