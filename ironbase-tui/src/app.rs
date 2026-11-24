//! Application state and navigation - Pane-based architecture
//!
//! State types are defined in the `state` module and re-exported here for convenience.

use crate::config::Config;
use crate::db::{CollectionInfo, DbWrapper};
use crate::i18n::{t, Op};
use crate::modals::confirm::{ConfirmAction, ConfirmOption, ConfirmState};
use crate::runtime::{Slot, Tasks, Update};
use crate::state::open_state;
use crate::theme::{Theme, ThemeName};
use ironbase_mcp_types::params::{
    AutoEmbedDisableParams, CollectionCreateParams, CollectionDropParams, CountParams,
    DeleteParams, ExplainParams, FindParams, IndexCreateArgs, IndexCreateParams, IndexDropParams,
    IndexListParams, InsertOneParams, JobCancelParams, JobListParams, NoParams, ScriptExecParams,
    ScriptHistoryParams, ScriptListParams, ScriptNameParams, ScriptSaveParams, ScriptVersionParams,
    UpdateOneParams,
};
use ironbase_mcp_types::responses::EmbedCacheClearResponse;
use ironbase_mcp_types::tools::{
    AutoEmbedDisable, CollectionCreate, CollectionDrop, Count, DbStats, DeleteOne, EmbedCacheClear,
    EmbedJobCancel, EmbedJobList, Explain, Find, IndexCreate, IndexDrop, IndexStatsRefresh,
    InsertOne, ScriptDelete, ScriptExec, ScriptGet, ScriptHistory, ScriptList, ScriptRollback,
    ScriptSave, UpdateOne,
};
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
    // Export
    ExportState,
    // Filter
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
    OpenModal,
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
    // Vector
    VectorSearchState,
    QUERY_TEMPLATES,
};

// === UI Layout Constants ===

/// Minimum document page size (keeps the pagination maths away from zero)
const MIN_PAGE_SIZE: usize = 1;

/// Detail pane scroll step (also used for page jump in collections)
const SCROLL_STEP: usize = 10;

/// `$get` / `$get_mut`: the open modal's state when the open modal is
/// `OpenModal::$variant`, else `None`
macro_rules! modal_state {
    ($get:ident, $get_mut:ident, $variant:ident, $state:ty) => {
        pub fn $get(&self) -> Option<&$state> {
            match &self.open_modal {
                Some(OpenModal::$variant(state)) => Some(state),
                _ => None,
            }
        }

        pub fn $get_mut(&mut self) -> Option<&mut $state> {
            match &mut self.open_modal {
                Some(OpenModal::$variant(state)) => Some(state),
                _ => None,
            }
        }
    };
}

/// Main application state
pub struct App {
    // Quit flag
    pub should_quit: bool,

    // Focus management
    pub active_pane: Pane,
    /// The open modal and its state; handlers reach a modal's state through
    /// this field (not a method) so `db` stays usable next to it
    pub(crate) open_modal: Option<OpenModal>,
    pub help_scroll: usize,

    // Theme
    pub theme_name: ThemeName,
    pub theme: Theme,

    // Database
    pub db: Option<DbWrapper>,
    /// Requests running in the background (`crate::runtime`)
    pub tasks: Tasks,
    pub db_path: Option<PathBuf>,

    // Collections pane
    pub collections: Vec<CollectionInfo>,
    pub selected_collection: usize,

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

    /// The selected collection's index names, shown in the collections pane
    pub collection_indexes: Vec<String>,

    // Visual filter state
    pub filter_state: FilterState,

    // Server info modal state
    pub server_info_scroll: usize,

    // Update modal state

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
        let theme = Theme::for_terminal(theme_name);

        Self {
            should_quit: false,
            active_pane: Pane::Collections,
            open_modal: None,
            help_scroll: 0,
            theme_name,
            theme,
            db: None,
            tasks: Tasks::default(),
            db_path: None,
            collections: Vec::new(),
            selected_collection: 0,
            documents: Vec::new(),
            selected_document: 0,
            doc_scroll_offset: 0,
            page_size: 20,
            total_docs: 0,
            active_filter: None,
            active_sort: None,
            detail_scroll: 0,
            search: SearchState::new(),
            collection_indexes: Vec::new(),
            filter_state: FilterState::default(),
            server_info_scroll: 0,
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
        // The page is exactly the rows the documents pane draws (same layout
        // and row maths as the renderer), so no fetched row is ever invisible
        let new_page_size =
            crate::panes::documents::visible_rows(crate::ui::content_height(terminal_height))
                .max(MIN_PAGE_SIZE);

        if new_page_size != self.page_size {
            self.page_size = new_page_size;
            // Keep the selected row inside the (possibly smaller) window
            if self.selected_document >= self.doc_scroll_offset + self.page_size {
                self.doc_scroll_offset = self.selected_document + 1 - self.page_size;
            }
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
        // While the list reloads its rows are the old page: nothing is selected
        if self.documents_loading() {
            return None;
        }
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
        &self.collection_indexes
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
        // Page of the last visible row: the window scrolls row by row, so the
        // offset is not page-aligned and the last window must show N/N
        let last_visible = (self.doc_scroll_offset + self.page_size)
            .min(self.total_docs)
            .saturating_sub(1);
        let current_page = (last_visible / self.page_size) + 1;
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
        self.set_modal(Modal::Search);
        self.search.input_active = true;
        self.search.query_input.clear();
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
        self.set_modal(Modal::Actions);
    }

    /// Open help modal
    pub fn open_help(&mut self) {
        self.set_modal(Modal::Help);
    }

    /// Open server info modal
    pub fn open_server_info(&mut self) {
        self.server_info_scroll = 0;
        self.open_modal = Some(OpenModal::ServerInfo(
            crate::modals::server_info::ServerInfoState::new(),
        ));
    }

    /// Update server info state with data from MCP server
    pub fn update_server_info(
        &mut self,
        db_stats: ironbase_mcp_types::responses::DbStatsResponse,
        tools: Vec<serde_json::Value>,
        prompts: Vec<serde_json::Value>,
    ) {
        if let Some(OpenModal::ServerInfo(state)) = &mut self.open_modal {
            state.update(db_stats, tools, prompts);
        }
    }

    /// Set server info error
    pub fn set_server_info_error(&mut self, error: String) {
        if let Some(OpenModal::ServerInfo(state)) = &mut self.open_modal {
            state.set_error(error);
        }
    }

    /// Open update check modal
    pub fn open_update(&mut self, current_version: String) {
        self.open_modal = Some(OpenModal::Update(crate::modals::update::UpdateState::new(
            current_version,
        )));
    }

    /// Update the update state with GitHub data
    pub fn update_update_state(
        &mut self,
        latest_version: String,
        download_url: String,
        release_notes: Option<String>,
    ) {
        if let Some(OpenModal::Update(state)) = &mut self.open_modal {
            state.update_from_github(latest_version, download_url, release_notes);
        }
    }

    /// Set update check error
    pub fn set_update_error(&mut self, error: String) {
        if let Some(OpenModal::Update(state)) = &mut self.open_modal {
            state.set_error(error);
        }
    }

    /// Close current modal
    pub fn close_modal(&mut self) {
        self.open_modal = None;
    }

    /// The open modal's kind
    pub fn modal(&self) -> Option<Modal> {
        self.open_modal.as_ref().map(OpenModal::kind)
    }

    /// The open modal with its state
    pub fn open_modal(&self) -> Option<&OpenModal> {
        self.open_modal.as_ref()
    }

    pub fn open_modal_mut(&mut self) -> Option<&mut OpenModal> {
        self.open_modal.as_mut()
    }

    modal_state!(export_state, export_state_mut, Export, ExportState);
    modal_state!(fulltext_state, fulltext_state_mut, Fulltext, FulltextState);
    modal_state!(database_state, database_state_mut, Database, DatabaseState);
    modal_state!(insert_state, insert_state_mut, Insert, InsertState);
    modal_state!(query_state, query_state_mut, Query, QueryState);
    modal_state!(index_state, index_state_mut, Index, IndexState);
    modal_state!(vector_state, vector_state_mut, Vector, VectorSearchState);
    modal_state!(script_state, script_state_mut, Script, ScriptState);
    modal_state!(rag_state, rag_state_mut, Rag, RagState);
    modal_state!(
        api_key_state,
        api_key_state_mut,
        ApiKey,
        crate::modals::api_key::ApiKeyState
    );
    modal_state!(acl_state, acl_state_mut, Acl, crate::modals::acl::AclState);
    modal_state!(
        listener_state,
        listener_state_mut,
        Listener,
        crate::modals::listener::ListenerState
    );

    /// Open a modal of `kind` with an empty state: for the modals without
    /// state of their own (the others have their own `open_*`, which fills it
    /// in). Never a modal without the state its keys and screen need.
    pub fn set_modal(&mut self, kind: Modal) {
        self.open_modal = Some(OpenModal::new_default(kind));
    }

    // === Delete Document ===

    /// Open delete confirmation for selected document
    pub fn open_delete_confirm(&mut self) {
        // Get current document and collection
        let Some(doc) = self.get_selected_document().cloned() else {
            self.set_error(t().no_document_selected());
            return;
        };

        let Some(coll_name) = self.current_collection_name().map(String::from) else {
            self.set_error(t().no_collection_selected());
            return;
        };

        let doc_id = doc.get("_id").cloned().unwrap_or(Value::Null);
        let preview = serde_json::to_string_pretty(&doc).unwrap_or_else(|_| "{}".to_string());

        self.open_confirm(ConfirmState::delete_document(coll_name, doc_id, &preview));
    }

    /// Open delete confirmation for selected collection
    pub fn open_delete_collection_confirm(&mut self) {
        let Some(coll) = self.collections.get(self.selected_collection).cloned() else {
            self.set_error(t().no_collection_selected());
            return;
        };

        let doc_count = coll.doc_count.unwrap_or(0);
        self.open_confirm(ConfirmState::delete_collection(coll.name, doc_count));
    }

    /// Open drop confirmation for the selected index (index modal)
    pub fn open_drop_index_confirm(&mut self) {
        let state = open_state!(self, Index);
        let Some(index_name) = state.indexes.get(state.selected_index).cloned() else {
            return;
        };

        if state.is_primary_index(&index_name) {
            state.error = Some(t().primary_index_not_droppable(&index_name));
            return;
        }

        let collection = state.collection.clone();
        self.open_confirm(ConfirmState::drop_index(collection, index_name));
    }

    /// Open drop confirmation for the selected vector index (vector modal)
    pub fn open_drop_vector_index_confirm(&mut self) {
        let state = open_state!(self, Vector);
        let Some(index) = state.indexes.get(state.selected_index) else {
            return;
        };

        let index_name = index.name.clone();
        let collection = state.collection.clone();
        self.open_confirm(ConfirmState::drop_vector_index(collection, index_name));
    }

    /// Open rollback confirmation for the selected script version (script history)
    pub fn open_rollback_script_confirm(&mut self) {
        let state = open_state!(self, Script);
        let Some(version_info) = state.versions.get(state.selected_version) else {
            state.error = Some(t().no_version_selected().to_string());
            return;
        };

        let version = version_info.version;
        let name = state.name.value().to_string();
        self.open_confirm(ConfirmState::rollback_script(name, version));
    }

    /// Open confirmation for disabling auto-embed (RAG modal)
    pub fn open_disable_auto_embed_confirm(&mut self) {
        let collection = open_state!(self, Rag).collection.clone();
        self.open_confirm(ConfirmState::disable_auto_embed(collection));
    }

    /// Open confirmation for clearing the embedding cache (RAG modal)
    pub fn open_clear_embed_cache_confirm(&mut self) {
        self.open_confirm(ConfirmState::clear_embed_cache());
    }

    /// Open confirmation for cancelling the selected embed job (RAG modal)
    pub fn open_cancel_embed_job_confirm(&mut self) {
        let Some(job_id) = open_state!(self, Rag).selected_job_id() else {
            return;
        };

        self.open_confirm(ConfirmState::cancel_embed_job(job_id));
    }

    /// Open a confirm dialog over the open modal (if any), which it keeps
    /// and gives back when it closes
    pub fn open_confirm(&mut self, state: ConfirmState) {
        let previous = self.open_modal.take().map(Box::new);
        self.open_modal = Some(OpenModal::Confirm { state, previous });
    }

    /// The open confirm dialog's state
    pub fn confirm_state(&self) -> Option<&ConfirmState> {
        match &self.open_modal {
            Some(OpenModal::Confirm { state, .. }) => Some(state),
            _ => None,
        }
    }

    pub fn confirm_state_mut(&mut self) -> Option<&mut ConfirmState> {
        match &mut self.open_modal {
            Some(OpenModal::Confirm { state, .. }) => Some(state),
            _ => None,
        }
    }

    /// Close the confirm dialog and return to the modal it was opened from,
    /// as it was (or to the panes)
    pub fn close_confirm(&mut self) {
        // Only the confirm dialog: another open modal stays as it is
        if matches!(self.open_modal, Some(OpenModal::Confirm { .. })) {
            if let Some(OpenModal::Confirm { previous, .. }) = self.open_modal.take() {
                self.open_modal = previous.map(|modal| *modal);
            }
        }
    }

    /// Toggle confirm dialog selection
    pub fn confirm_toggle(&mut self) {
        if let Some(state) = self.confirm_state_mut() {
            state.toggle_selection();
        }
    }

    // execute_confirm_action, do_delete_document, execute_insert törölve
    // Használd: execute_confirm_action_async, execute_insert_async

    // === Insert Document ===

    /// Open insert document modal
    pub fn open_insert(&mut self) {
        let Some(coll_name) = self.current_collection_name().map(String::from) else {
            self.set_error(t().no_collection_selected());
            return;
        };

        // Use template from existing document or default
        let template = if let Some(doc) = self.documents.first() {
            Self::create_empty_template(doc)
        } else {
            None
        };

        self.open_modal = Some(OpenModal::Insert(InsertState::with_template(
            coll_name, template,
        )));
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
            self.set_error(t().no_document_selected());
            return;
        };

        let Some(coll_name) = self.current_collection_name().map(String::from) else {
            self.set_error(t().no_collection_selected());
            return;
        };

        self.open_modal = Some(OpenModal::Insert(InsertState::edit(coll_name, &doc)));
    }

    // === Index Management ===

    /// Open index management modal (sync, deferred index loading)
    pub fn open_index_modal(&mut self) {
        let Some(coll_name) = self.current_collection_name().map(String::from) else {
            self.set_error(t().no_collection_selected());
            return;
        };

        // Initialize with empty, will be loaded async
        self.open_modal = Some(OpenModal::Index(IndexState::new(coll_name, vec![])));
    }

    /// Load indexes for current collection (call after opening index modal)
    pub async fn load_indexes_async(&mut self) {
        let collection = open_state!(self, Index).collection.clone();
        if collection.is_empty() {
            return;
        }

        let Some(db) = &self.db else {
            return;
        };

        match db.index_entries(&collection).await {
            Ok(entries) => {
                open_state!(self, Index).indexes = entries.iter().map(|e| e.name.clone()).collect();
                self.collection_indexes = open_state!(self, Index).indexes.clone();
                open_state!(self, Index).index_entries = entries;
            }
            Err(e) => {
                open_state!(self, Index).message = Some(t().failed(Op::LoadIndexes, &e));
            }
        }
    }

    // execute_create_index, execute_delete_index törölve
    // Használd az async verziókat: execute_create_index_async, execute_delete_index_async

    // === Query Builder ===

    /// Open query builder modal
    pub fn open_query_modal(&mut self) {
        let Some(coll_name) = self.current_collection_name().map(String::from) else {
            self.set_error(t().no_collection_selected());
            return;
        };

        self.open_modal = Some(OpenModal::Query(QueryState::new(coll_name)));
    }

    /// Open visual filter modal with schema loading (async)
    pub async fn open_filter_modal_async(&mut self) {
        let coll_name = match self.current_collection_name() {
            Some(name) => name.to_string(),
            None => {
                self.set_error(t().no_collection_selected());
                return;
            }
        };

        // Preserve existing filters, only reset input fields
        self.filter_state.reset_inputs();
        self.set_modal(Modal::Filter);

        // Load schema for field suggestions
        if let Some(db) = &self.db {
            match db.infer_schema(&coll_name, 100).await {
                Ok(fields) => {
                    let field_names: Vec<String> = fields.into_iter().map(|f| f.name).collect();
                    self.filter_state.set_fields(field_names);
                }
                Err(e) => {
                    // Non-fatal - just no suggestions
                    self.filter_state.error = Some(t().failed(Op::LoadSchema, &e));
                }
            }
        }
    }

    // execute_query törölve - használd execute_query_async

    // === Export ===

    /// Open export modal
    pub fn open_export_modal(&mut self) {
        let Some(coll) = self.collections.get(self.selected_collection) else {
            self.set_error(t().no_collection_selected());
            return;
        };

        let mut state = ExportState::new(coll.name.clone(), coll.doc_count.unwrap_or(0));
        // The export follows the document list: with an active filter the
        // count is the filtered one
        if self.active_filter.is_some() {
            state.doc_count = self.total_docs;
            state.filtered = true;
        }
        self.open_modal = Some(OpenModal::Export(state));
    }

    // === New Collection ===

    /// Open new collection modal
    pub fn open_new_collection(&mut self) {
        self.open_modal = Some(OpenModal::NewCollection(NewCollectionState::new()));
    }

    // === IronRhai Scripts ===

    /// Open script modal (Browse mode)
    pub fn open_script_modal(&mut self) {
        self.open_modal = Some(OpenModal::Script(ScriptState::new()));
        // Script list will be loaded in load_scripts_async
    }

    /// Load scripts from MCP server
    pub async fn load_scripts_async(&mut self) {
        let Some(db) = &self.db else {
            open_state!(self, Script).error = Some(t().no_database().to_string());
            return;
        };

        match db.call::<ScriptList>(&ScriptListParams::default()).await {
            Ok(response) => {
                open_state!(self, Script).scripts = response.scripts;
                open_state!(self, Script).selected_script = 0;
                open_state!(self, Script).error = None;
            }
            Err(e) => {
                open_state!(self, Script).error = Some(t().failed(Op::LoadScripts, &e));
            }
        }
    }

    /// Load a script for editing by name
    pub async fn load_script_for_edit_async(&mut self) {
        let state = open_state!(self, Script);
        let Some(script_info) = state.scripts.get(state.selected_script) else {
            return;
        };
        let script_name = script_info.name.clone();

        let Some(db) = &self.db else {
            open_state!(self, Script).error = Some(t().no_database().to_string());
            return;
        };

        let args = ScriptNameParams {
            name: script_name.clone(),
        };
        match db.call::<ScriptGet>(&args).await {
            Ok(script) => {
                open_state!(self, Script).load_script(
                    script_name,
                    script.code,
                    script.description,
                    script.tags,
                    script.version,
                );
            }
            Err(e) => {
                open_state!(self, Script).error = Some(t().failed(Op::LoadScript, &e));
            }
        }
    }

    /// Save current script to MCP server
    pub async fn save_script_async(&mut self) {
        if open_state!(self, Script).name.value().trim().is_empty() {
            open_state!(self, Script).error = Some(t().script_name_required().to_string());
            return;
        }

        let Some(db) = &self.db else {
            open_state!(self, Script).error = Some(t().no_database().to_string());
            return;
        };

        let args = ScriptSaveParams {
            name: open_state!(self, Script).name.value().to_string(),
            code: open_state!(self, Script).editor.text(),
            description: Some(open_state!(self, Script).description.value().to_string())
                .filter(|description| !description.is_empty()),
            tags: Some(open_state!(self, Script).tags.clone()).filter(|tags| !tags.is_empty()),
            dependencies: None,
        };

        match db.call::<ScriptSave>(&args).await {
            Ok(saved) => {
                let new_version = saved.version;
                open_state!(self, Script).version = new_version;
                open_state!(self, Script).dirty = false;
                open_state!(self, Script).message = Some(t().script_saved(new_version));
                open_state!(self, Script).error = None;
                // Reload script list to show new/updated scripts
                self.load_scripts_async().await;
            }
            Err(e) => {
                open_state!(self, Script).error = Some(t().failed(Op::Save, &e));
            }
        }
    }

    /// Delete selected script
    pub async fn delete_script_async(&mut self) {
        let state = open_state!(self, Script);
        let Some(script_info) = state.scripts.get(state.selected_script) else {
            return;
        };
        let script_name = script_info.name.clone();

        let Some(db) = &self.db else {
            open_state!(self, Script).error = Some(t().no_database().to_string());
            return;
        };

        let args = ScriptNameParams {
            name: script_name.clone(),
        };
        match db.call::<ScriptDelete>(&args).await {
            Ok(_) => {
                open_state!(self, Script).message = Some(t().script_deleted(&script_name));
                // Reload list
                self.load_scripts_async().await;
            }
            Err(e) => {
                open_state!(self, Script).error = Some(t().failed(Op::Delete, &e));
            }
        }
    }

    /// Run script (saved or inline)
    pub async fn run_script_async(&mut self) {
        let Some(db) = &self.db else {
            open_state!(self, Script).error = Some(t().no_database().to_string());
            return;
        };

        // Parse params
        let params: Option<serde_json::Value> = if open_state!(self, Script)
            .params_input
            .value()
            .trim()
            .is_empty()
            || open_state!(self, Script).params_input.value().trim() == "{}"
        {
            None
        } else {
            match serde_json::from_str(open_state!(self, Script).params_input.value()) {
                Ok(v) => Some(v),
                Err(e) => {
                    open_state!(self, Script).error = Some(t().invalid_json_params(&e));
                    return;
                }
            }
        };

        let start = std::time::Instant::now();

        // Always execute current editor content (not saved version)
        // This ensures "what you see is what runs"
        let args = ScriptExecParams {
            code: open_state!(self, Script).editor.text(),
            params,
            max_operations: None,
        };
        let result = db.call::<ScriptExec>(&args).await;

        // A failed script is a tool error: the server answers a run with its
        // own result, logs and execution time
        match result {
            Ok(response) => {
                open_state!(self, Script).result = Some(ScriptResult {
                    success: true,
                    result: response.run.result,
                    logs: response.run.logs,
                    error: None,
                    execution_time_ms: response.run.execution_time_ms,
                });
                open_state!(self, Script).error = None;
            }
            Err(e) => {
                open_state!(self, Script).result = Some(ScriptResult {
                    success: false,
                    result: serde_json::Value::Null,
                    logs: vec![],
                    error: Some(e.to_string()),
                    execution_time_ms: start.elapsed().as_millis() as u64,
                });
            }
        }
    }

    /// Load script version history
    pub async fn load_script_history_async(&mut self) {
        let script_name = open_state!(self, Script).name.value().to_string();
        if script_name.is_empty() || script_name == "[inline]" {
            open_state!(self, Script).error = Some(t().no_script_selected().to_string());
            return;
        }

        let Some(db) = &self.db else {
            open_state!(self, Script).error = Some(t().no_database().to_string());
            return;
        };

        let args = ScriptHistoryParams {
            name: script_name,
            limit: Some(20),
        };
        match db.call::<ScriptHistory>(&args).await {
            Ok(response) => {
                open_state!(self, Script).versions = response.history;
                open_state!(self, Script).selected_version = 0;
                open_state!(self, Script).error = None;
            }
            Err(e) => {
                open_state!(self, Script).error = Some(t().failed(Op::LoadHistory, &e));
            }
        }
    }

    /// Rollback script to a specific version - name/version are captured when the
    /// confirm dialog opens
    pub async fn rollback_script_async(&mut self, script_name: &str, target_version: u32) {
        let Some(db) = &self.db else {
            open_state!(self, Script).error = Some(t().no_database().to_string());
            return;
        };

        let args = ScriptVersionParams {
            name: script_name.to_string(),
            version: target_version,
        };
        match db
            .call::<ScriptRollback>(&args)
            .await
            .map(|r| r.new_version)
        {
            Ok(new_version) => {
                open_state!(self, Script).message = Some(t().script_rolled_back(new_version));
                // Reload the script into editor
                self.load_script_for_edit_async().await;
            }
            Err(e) => {
                open_state!(self, Script).error = Some(t().failed(Op::Rollback, &e));
            }
        }
    }

    fn set_new_collection_error(&mut self, error: String) {
        if let Some(OpenModal::NewCollection(state)) = &mut self.open_modal {
            state.error = Some(error);
        }
    }

    /// Create new collection (async)
    pub async fn create_collection_async(&mut self) {
        let Some(OpenModal::NewCollection(state)) = &self.open_modal else {
            return;
        };
        let name = state.name.value().trim().to_string();
        if name.is_empty() {
            self.set_new_collection_error(t().name_required().to_string());
            return;
        }

        // Check if collection already exists
        if self.collections.iter().any(|c| c.name == name) {
            self.set_new_collection_error(t().collection_exists().to_string());
            return;
        }

        let Some(db) = &self.db else {
            self.set_error(t().no_database());
            return;
        };

        let args = CollectionCreateParams {
            collection: name.clone(),
        };
        match db.call::<CollectionCreate>(&args).await {
            Ok(_) => {
                self.close_modal();
                self.set_status(t().collection_created(&name));
                self.reload_collections_async().await;
                // Select the new collection
                if let Some(idx) = self.collections.iter().position(|c| c.name == name) {
                    self.switch_collection(idx);
                }
            }
            Err(e) => {
                self.set_new_collection_error(t().failed(Op::Create, &e));
            }
        }
    }

    // === Theme ===

    /// Cycle to next theme
    pub fn next_theme(&mut self) {
        self.theme_name = self.theme_name.next();
        self.theme = Theme::for_terminal(self.theme_name);
        self.set_status(t().theme_changed(self.theme_name.name()));
    }

    // === Status ===

    /// Set status message with auto-clear timestamp
    ///
    /// Also drops a stale error: the status bar prefers `error_message`, so an
    /// earlier error would otherwise hide every later success.
    pub fn set_status(&mut self, msg: impl Into<String>) {
        self.error_message = None;
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
        let msg = msg.into();
        tracing::warn!(
            error = %crate::logging::sanitize_for_log(&msg),
            "Error shown to the user"
        );
        self.error_message = Some(msg);
    }

    /// Reset UI state for database switch (clear documents, filters, selections)
    pub fn reset_for_new_database(&mut self) {
        self.tasks.cancel(Slot::Documents);
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
            self.set_error(t().no_document_selected());
            return;
        };

        let json = serde_json::to_string_pretty(doc).unwrap_or_else(|_| "{}".to_string());

        match arboard::Clipboard::new() {
            Ok(mut clipboard) => match clipboard.set_text(&json) {
                Ok(()) => {
                    let preview: String = json.chars().take(50).collect();
                    self.set_status(t().copied(&preview));
                }
                Err(e) => {
                    self.set_error(t().failed(Op::Copy, &e));
                }
            },
            Err(e) => {
                self.set_error(t().clipboard_unavailable(&e));
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
            let stats = db.call::<DbStats>(&NoParams {}).await?;
            self.db_path = Some(std::path::PathBuf::from(stats.stats.database.path));
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
            // Collection identity is the NAME: the index is only valid for one list
            let previous = self.current_collection_name().map(String::from);
            self.collections = collections;
            self.reselect_collection(previous.as_deref());
            self.refresh_documents_inner().await?;
        }
        Ok(())
    }

    /// Re-resolve the selection after the collection list was replaced.
    ///
    /// Keeps the previously selected collection (by name). If it no longer
    /// exists, falls back to a clamped index and drops the filter/sort and the
    /// document position, which belonged to the old collection.
    /// Returns true if the same collection is still selected.
    fn reselect_collection(&mut self, previous: Option<&str>) -> bool {
        if let Some(idx) =
            previous.and_then(|name| self.collections.iter().position(|c| c.name == name))
        {
            self.selected_collection = idx;
            return true;
        }
        let clamped = self
            .selected_collection
            .min(self.collections.len().saturating_sub(1));
        self.set_selected_collection(clamped);
        false
    }

    /// Select a collection by index: the single place that switches
    /// collection. The filter, sort and document position belong to the old
    /// collection, so they are always dropped (the caller reloads documents).
    fn set_selected_collection(&mut self, idx: usize) {
        self.selected_collection = idx;
        self.doc_scroll_offset = 0;
        self.selected_document = 0;
        self.active_filter = None; // Clear filter on collection change
        self.active_sort = None; // Clear sort on collection change
        self.filter_state = FilterState::new(); // Clear filter state too
    }

    /// Switch to another collection and load its documents (in the background)
    fn switch_collection(&mut self, idx: usize) {
        self.set_selected_collection(idx);
        self.reload_documents();
    }

    /// Drop the loaded documents (nothing is selected, nothing is shown)
    fn clear_documents(&mut self) {
        self.tasks.cancel(Slot::Documents);
        self.documents.clear();
        self.selected_document = 0;
        self.doc_scroll_offset = 0;
        self.total_docs = 0;
    }

    /// Reload the document list in the background: the screen keeps
    /// drawing meanwhile (the old page stays shown, but nothing is selected
    /// until the new one arrives). A failure clears the list and is shown.
    pub fn reload_documents(&mut self) {
        let (Some(db), Some(request)) = (self.db.clone(), self.documents_request()) else {
            self.tasks.cancel(Slot::Documents);
            return;
        };
        self.tasks.spawn(Slot::Documents, async move {
            let page = fetch_documents(&db, &request).await;
            Box::new(move |app: &mut App| {
                // The view changed meanwhile without a new load: not this page
                if app.documents_request().as_ref() != Some(&request) {
                    return;
                }
                if let Err(e) = app.apply_documents(page) {
                    app.set_error(t().failed(Op::LoadDocuments, &e));
                }
            }) as Update
        });
    }

    /// Stop the background loads. A stopped document load leaves the old
    /// page's rows under the new position, so the list is cleared
    pub fn cancel_tasks(&mut self) {
        let documents = self.documents_loading();
        if self.tasks.cancel_all() {
            if documents {
                self.clear_documents();
            }
            self.set_status(t().tasks_cancelled());
        }
    }

    /// Whether the document list is being reloaded (its rows are the old page)
    pub fn documents_loading(&self) -> bool {
        self.tasks.is_running(Slot::Documents)
    }

    /// Refresh collections + documents and surface a failure
    async fn reload_collections_async(&mut self) {
        if let Err(e) = self.refresh_collections_async().await {
            self.set_error(t().failed(Op::Refresh, &e));
        }
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

    /// Refresh documents and wait for them; on failure the list is cleared,
    /// so the panes never show (and `d`/`e` never act on) another
    /// collection's or page's documents. A background reload still running
    /// is superseded.
    async fn refresh_documents_inner(&mut self) -> anyhow::Result<()> {
        self.tasks.cancel(Slot::Documents);
        let (Some(db), Some(request)) = (self.db.as_ref(), self.documents_request()) else {
            if self.collections.get(self.selected_collection).is_none() {
                self.clear_documents();
            }
            return Ok(());
        };
        let page = fetch_documents(db, &request).await;
        self.apply_documents(page)
    }

    /// What the document list shows now: the page to load for it
    fn documents_request(&self) -> Option<DocumentsRequest> {
        let collection = self.collections.get(self.selected_collection)?.name.clone();
        Some(DocumentsRequest {
            collection,
            filter: self.active_filter.clone(),
            sort: self.active_sort.clone(),
            skip: self.doc_scroll_offset,
            limit: self.page_size,
        })
    }

    /// Show a loaded page (or clear the list on failure)
    fn apply_documents(&mut self, page: anyhow::Result<DocumentsPage>) -> anyhow::Result<()> {
        let page = match page {
            Ok(page) => page,
            Err(e) => {
                self.clear_documents();
                return Err(e);
            }
        };
        self.documents = page.documents;
        self.total_docs = page.total;
        if let Some((doc_count, index_names)) = page.details {
            // Update cached collection info
            if let Some(coll) = self.collections.get_mut(self.selected_collection) {
                coll.doc_count = Some(doc_count);
                coll.index_count = Some(index_names.len());
            }
            self.collection_indexes = index_names;
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
        Ok(())
    }

    /// Get indexes for current collection (async)
    pub async fn get_current_indexes_async(&self) -> Vec<String> {
        if let Some(db) = &self.db {
            if let Some(coll) = self.collections.get(self.selected_collection) {
                return db
                    .index_entries(&coll.name)
                    .await
                    .map(|entries| entries.into_iter().map(|e| e.name).collect())
                    .unwrap_or_default();
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
                    self.switch_collection(self.selected_collection - 1);
                }
            }
            Pane::Documents => {
                if self.selected_document > 0 {
                    self.selected_document -= 1;
                    if self.selected_document < self.doc_scroll_offset {
                        self.doc_scroll_offset = self.selected_document;
                        self.reload_documents();
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
                    self.switch_collection(self.selected_collection + 1);
                }
            }
            Pane::Documents => {
                if self.selected_document + 1 < self.total_docs {
                    self.selected_document += 1;
                    let visible_end = self.doc_scroll_offset + self.page_size;
                    if self.selected_document >= visible_end {
                        self.doc_scroll_offset += 1;
                        self.reload_documents();
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
                let target = self.selected_collection.saturating_sub(SCROLL_STEP);
                if target != self.selected_collection {
                    self.switch_collection(target);
                }
            }
            Pane::Documents => {
                let old_offset = self.doc_scroll_offset;
                self.doc_scroll_offset = self.doc_scroll_offset.saturating_sub(self.page_size);
                self.selected_document = self.selected_document.saturating_sub(self.page_size);
                if old_offset != self.doc_scroll_offset {
                    self.reload_documents();
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
                let target = (self.selected_collection + SCROLL_STEP).min(max);
                if target != self.selected_collection {
                    self.switch_collection(target);
                }
            }
            Pane::Documents => {
                let max_offset = self.total_docs.saturating_sub(self.page_size);
                let old_offset = self.doc_scroll_offset;
                self.doc_scroll_offset = (self.doc_scroll_offset + self.page_size).min(max_offset);
                self.selected_document = (self.selected_document + self.page_size)
                    .min(self.total_docs.saturating_sub(1));
                if old_offset != self.doc_scroll_offset {
                    self.reload_documents();
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
                    self.switch_collection(0);
                }
            }
            Pane::Documents => {
                if self.doc_scroll_offset != 0 || self.selected_document != 0 {
                    self.doc_scroll_offset = 0;
                    self.selected_document = 0;
                    self.reload_documents();
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
                    self.switch_collection(last);
                }
            }
            Pane::Documents => {
                if self.total_docs == 0 {
                    return;
                }
                let last = self.total_docs - 1;
                self.selected_document = last;
                self.doc_scroll_offset = last.saturating_sub(self.page_size.saturating_sub(1));
                self.reload_documents();
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
        let query = self.search.query_input.value().trim();
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
            self.set_error(t().no_database());
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
                self.set_error(t().failed(Op::Search, &e));
            }
        }

        if self.search.results.is_empty() {
            self.set_status(t().no_matches());
        } else {
            self.set_status(t().match_count(self.search.results.len()));
        }
    }

    /// Search within current document (sync - local operation)
    pub fn execute_document_search(&mut self) {
        let query = self.search.query_input.value().trim().to_lowercase();
        if query.is_empty() {
            self.search.doc_matches.clear();
            return;
        }

        self.search.doc_matches.clear();
        self.search.current_match = 0;

        // Get current document
        let Some(doc) = self.get_selected_document() else {
            self.set_status(t().no_document_selected());
            return;
        };

        // Pretty print and search
        let pretty = serde_json::to_string_pretty(&doc).unwrap_or_default();

        for (line_num, line) in pretty.lines().enumerate() {
            let line_lower = line.to_lowercase();
            let mut search_start = 0;
            while let Some(col) = line_lower[search_start..].find(&query) {
                self.search
                    .doc_matches
                    .push(DocSearchMatch { line: line_num });
                search_start += col + query.len();
            }
        }

        if self.search.doc_matches.is_empty() {
            self.set_status(t().no_matches());
        } else {
            self.set_status(t().match_count_with_hint(self.search.doc_matches.len()));
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
            self.set_status(
                t().match_position(self.search.current_match + 1, self.search.doc_matches.len()),
            );
        }
    }

    pub fn goto_prev_match(&mut self) {
        self.search.prev_match();
        self.scroll_to_current_match();
        if !self.search.doc_matches.is_empty() {
            self.set_status(
                t().match_position(self.search.current_match + 1, self.search.doc_matches.len()),
            );
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
                self.switch_collection(idx);
                self.active_pane = Pane::Documents;
            }
            self.close_modal();
        }
    }

    // === Async Confirm ===

    /// Execute the confirmed action (async)
    pub async fn execute_confirm_action_async(&mut self) {
        let Some(state) = self.confirm_state() else {
            return;
        };
        let action = (state.selected == ConfirmOption::Confirm).then(|| state.on_confirm.clone());
        self.close_confirm();
        let Some(action) = action else {
            return;
        };

        match action {
            ConfirmAction::DeleteDocument { collection, doc_id } => {
                self.do_delete_document_async(&collection, &doc_id).await;
            }
            ConfirmAction::DeleteCollection { name } => {
                self.do_delete_collection_async(&name).await;
            }
            ConfirmAction::DropIndex {
                collection,
                index_name,
            } => {
                self.execute_delete_index_async(&collection, &index_name)
                    .await;
            }
            ConfirmAction::DropVectorIndex {
                collection,
                index_name,
            } => {
                self.do_drop_vector_index_async(&collection, &index_name)
                    .await;
            }
            ConfirmAction::RollbackScript { name, version } => {
                self.rollback_script_async(&name, version).await;
            }
            ConfirmAction::DisableAutoEmbed { collection } => {
                self.do_disable_auto_embed_async(&collection).await;
            }
            ConfirmAction::ClearEmbedCache => {
                self.do_clear_embed_cache_async().await;
            }
            ConfirmAction::CancelEmbedJob { job_id } => {
                self.do_cancel_embed_job_async(&job_id).await;
            }
        }
    }

    /// Actually drop a vector index (async)
    async fn do_drop_vector_index_async(&mut self, collection: &str, index_name: &str) {
        if let Some(ref db) = self.db {
            let args = IndexDropParams {
                collection: collection.to_string(),
                index_name: index_name.to_string(),
            };
            match db.call::<IndexDrop>(&args).await {
                Ok(dropped) => {
                    open_state!(self, Vector).message = Some(t().index_dropped(&dropped.dropped));

                    // Refresh index list
                    if let Ok(indexes) = db.vector_indexes(collection).await {
                        open_state!(self, Vector).update_indexes(indexes);
                        if open_state!(self, Vector).selected_index
                            >= open_state!(self, Vector).indexes.len()
                            && !open_state!(self, Vector).indexes.is_empty()
                        {
                            open_state!(self, Vector).selected_index =
                                open_state!(self, Vector).indexes.len() - 1;
                        }
                    }
                }
                Err(e) => {
                    open_state!(self, Vector).error = Some(format!("{}", e));
                }
            }
        }
    }

    /// Actually disable auto-embed for a collection (async)
    async fn do_disable_auto_embed_async(&mut self, collection: &str) {
        if let Some(ref db) = self.db {
            let args = AutoEmbedDisableParams {
                collection: collection.to_string(),
            };
            match db.call::<AutoEmbedDisable>(&args).await {
                Ok(_) => {
                    open_state!(self, Rag).message = Some(t().auto_embed_disabled().to_string());
                    open_state!(self, Rag).auto_embed_config = None;
                }
                Err(e) => {
                    open_state!(self, Rag).error = Some(format!("{}", e));
                }
            }
        }
    }

    /// Actually clear the embedding cache (async)
    async fn do_clear_embed_cache_async(&mut self) {
        if let Some(ref db) = self.db {
            match db.call::<EmbedCacheClear>(&NoParams {}).await {
                Ok(EmbedCacheClearResponse::Cleared { message, .. }) => {
                    open_state!(self, Rag).cache_stats = None;
                    open_state!(self, Rag).message = Some(message);
                }
                // No cache on the server: nothing was cleared
                Ok(EmbedCacheClearResponse::Disabled { message, .. }) => {
                    open_state!(self, Rag).error = Some(message);
                }
                Err(e) => {
                    open_state!(self, Rag).error = Some(format!("{}", e));
                }
            }
        }
    }

    /// Actually cancel an embed job (async)
    async fn do_cancel_embed_job_async(&mut self, job_id: &str) {
        if let Some(ref db) = self.db {
            let args = JobCancelParams {
                job_id: job_id.to_string(),
            };
            match db.call::<EmbedJobCancel>(&args).await {
                Ok(response) if response.success => {
                    open_state!(self, Rag).message = Some(response.message);
                }
                // Unknown or already finished: the server says which
                Ok(response) => open_state!(self, Rag).error = Some(response.message),
                Err(e) => {
                    open_state!(self, Rag).error = Some(format!("{}", e));
                    return;
                }
            }
            // The cancelled job's new state (or the finished one's)
            match db
                .call::<EmbedJobList>(&JobListParams { active_only: false })
                .await
            {
                Ok(result) => open_state!(self, Rag).set_jobs(result.jobs),
                Err(e) => open_state!(self, Rag).error = Some(t().failed(Op::LoadJobs, &e)),
            }
        }
    }

    /// Actually delete a document (async)
    async fn do_delete_document_async(&mut self, collection: &str, doc_id: &Value) {
        let Some(db) = &self.db else {
            self.set_error(t().no_database());
            return;
        };

        let args = DeleteParams {
            collection: collection.to_string(),
            filter: serde_json::json!({"_id": doc_id}),
        };
        match db.call::<DeleteOne>(&args).await {
            Ok(deleted) => {
                if deleted.deleted_count > 0 {
                    self.set_status(t().document_deleted());
                    self.reload_collections_async().await;
                } else {
                    self.set_error(t().document_not_found());
                }
            }
            Err(e) => {
                self.set_error(t().failed(Op::Delete, &e));
            }
        }
    }

    /// Actually delete a collection (async)
    async fn do_delete_collection_async(&mut self, name: &str) {
        let Some(db) = &self.db else {
            self.set_error(t().no_database());
            return;
        };

        let args = CollectionDropParams {
            collection: name.to_string(),
        };
        match db.call::<CollectionDrop>(&args).await {
            Ok(dropped) => {
                self.set_status(t().collection_dropped(&dropped.dropped));
                // Reset selection (also drops the dropped collection's
                // filter/sort) and refresh
                self.set_selected_collection(0);
                self.clear_documents();
                self.reload_collections_async().await;
            }
            Err(e) => {
                self.set_error(t().failed(Op::Delete, &e));
            }
        }
    }

    // === Async Insert/Edit ===

    /// Execute the insert/edit operation (async)
    pub async fn execute_insert_async(&mut self) {
        let doc = match open_state!(self, Insert).parse_json() {
            Ok(doc) => doc,
            Err(e) => {
                open_state!(self, Insert).error = Some(t().invalid_json(&e));
                return;
            }
        };

        if !doc.is_object() {
            open_state!(self, Insert).error = Some(t().document_must_be_object().to_string());
            return;
        }

        let collection = open_state!(self, Insert).collection.clone();
        let Some(db) = &self.db else {
            self.set_error(t().no_database());
            return;
        };

        if open_state!(self, Insert).is_edit_mode() {
            let Some(ref doc_id) = open_state!(self, Insert).original_doc_id else {
                open_state!(self, Insert).error = Some(t().document_id_missing().to_string());
                return;
            };

            let args = UpdateOneParams {
                collection,
                filter: serde_json::json!({"_id": doc_id}),
                update: open_state!(self, Insert).edit_update(&doc),
                upsert: false,
            };
            match db.call::<UpdateOne>(&args).await {
                Ok(updated) => {
                    self.close_modal();
                    if updated.matched_count == 0 {
                        self.set_status(t().document_not_found());
                    } else if let Some(err) = updated.auto_embed_error {
                        // The update is stored; only the vector is stale
                        self.set_error(t().document_updated_vector_stale(&err));
                    } else if updated.modified_count == 0 {
                        self.set_status(t().document_unchanged());
                    } else {
                        self.set_status(t().document_updated());
                    }
                    self.reload_collections_async().await;
                }
                Err(e) => {
                    open_state!(self, Insert).error = Some(t().failed(Op::Update, &e));
                }
            }
        } else {
            let args = InsertOneParams {
                collection,
                document: doc,
            };
            match db.call::<InsertOne>(&args).await {
                Ok(inserted) => {
                    self.close_modal();
                    self.set_status(t().document_inserted(&inserted.inserted_id));
                    self.reload_collections_async().await;
                }
                Err(e) => {
                    open_state!(self, Insert).error = Some(t().failed(Op::Insert, &e));
                }
            }
        }
    }

    // === Async Index Management ===

    /// Create a new index (async) - supports both single and compound indexes
    pub async fn execute_create_index_async(&mut self) {
        let fields = open_state!(self, Index).get_fields();

        if fields.is_empty() {
            open_state!(self, Index).error = Some(t().field_name_required().to_string());
            return;
        }

        let collection = open_state!(self, Index).collection.clone();
        let unique = open_state!(self, Index).unique;
        let sparse = open_state!(self, Index).sparse;
        let is_compound = open_state!(self, Index).is_compound;

        let Some(db) = &self.db else {
            self.set_error(t().no_database());
            return;
        };

        let (field, compound_fields) = if is_compound && fields.len() > 1 {
            // Compound index
            (None, Some(fields.clone()))
        } else {
            // Single field index
            (Some(fields.first().cloned().unwrap_or_default()), None)
        };
        let args = IndexCreateArgs::BTree(IndexCreateParams {
            collection,
            field,
            fields: compound_fields,
            unique,
            sparse,
        });

        match db.call::<IndexCreate>(&args).await {
            Ok(created) => {
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

                let compound = is_compound && fields.len() > 1;
                let fields = if compound {
                    fields.join(", ")
                } else {
                    fields.first().cloned().unwrap_or_default()
                };
                open_state!(self, Index).message =
                    Some(t().index_created(&fields, compound, &flags_str, created.index_name()));
                open_state!(self, Index).cancel_create();
                open_state!(self, Index).indexes = self.get_current_indexes_async().await;
                self.collection_indexes = open_state!(self, Index).indexes.clone();
                // Refresh collection list to update index_count
                self.reload_collections_async().await;
            }
            Err(e) => {
                open_state!(self, Index).error = Some(t().failed(Op::Create, &e));
            }
        }
    }

    /// Delete an index (async) - names are captured when the confirm dialog opens
    pub async fn execute_delete_index_async(&mut self, collection: &str, index_name: &str) {
        let Some(db) = &self.db else {
            self.set_error(t().no_database());
            return;
        };

        let args = IndexDropParams {
            collection: collection.to_string(),
            index_name: index_name.to_string(),
        };
        match db.call::<IndexDrop>(&args).await {
            Ok(dropped) => {
                open_state!(self, Index).message = Some(t().index_dropped(&dropped.dropped));
                open_state!(self, Index).error = None;
                open_state!(self, Index).indexes = self.get_current_indexes_async().await;
                self.collection_indexes = open_state!(self, Index).indexes.clone();
                // Adjust selection after deletion
                if open_state!(self, Index).indexes.is_empty() {
                    open_state!(self, Index).selected_index = 0;
                } else if open_state!(self, Index).selected_index
                    >= open_state!(self, Index).indexes.len()
                {
                    open_state!(self, Index).selected_index =
                        open_state!(self, Index).indexes.len() - 1;
                }
                // Refresh collection list to update index_count
                self.reload_collections_async().await;
            }
            Err(e) => {
                open_state!(self, Index).error = Some(t().failed(Op::Delete, &e));
            }
        }
    }

    /// Analyze indexes - refresh statistics (async)
    pub async fn execute_analyze_index_async(&mut self) {
        let collection = open_state!(self, Index).collection.clone();

        let Some(db) = &self.db else {
            self.set_error(t().no_database());
            return;
        };

        // Set analyzing state
        open_state!(self, Index).is_analyzing = true;
        open_state!(self, Index).message = Some(t().analyzing_indexes().to_string());

        // Refresh statistics
        let args = IndexListParams {
            collection: collection.clone(),
            index_type: None,
        };
        match db.call::<IndexStatsRefresh>(&args).await {
            Ok(_) => {
                // Load updated statistics
                match db.btree_index_statistics(&collection).await {
                    Ok(stats) => {
                        open_state!(self, Index).update_statistics(stats);
                        open_state!(self, Index).message =
                            Some(t().index_statistics_refreshed().to_string());
                    }
                    Err(e) => {
                        open_state!(self, Index).is_analyzing = false;
                        open_state!(self, Index).error = Some(t().failed(Op::LoadStatistics, &e));
                    }
                }
            }
            Err(e) => {
                open_state!(self, Index).is_analyzing = false;
                open_state!(self, Index).error = Some(t().failed(Op::Analyze, &e));
            }
        }
    }

    /// Load index statistics (async) - call when opening index modal
    pub async fn load_index_statistics_async(&mut self) {
        let collection = open_state!(self, Index).collection.clone();

        let Some(db) = &self.db else {
            return;
        };

        if let Ok(stats) = db.btree_index_statistics(&collection).await {
            open_state!(self, Index).update_statistics(stats);
        }
    }

    // === Async Query ===

    /// Execute the query (async) - max 1000 results
    pub async fn execute_query_async(&mut self) {
        const QUERY_LIMIT: usize = 1000;

        let query = match open_state!(self, Query).parse_query() {
            Ok(q) => q,
            Err(e) => {
                open_state!(self, Query).error = Some(t().invalid_json(&e));
                return;
            }
        };

        let collection = open_state!(self, Query).collection.clone();

        let Some(db) = &self.db else {
            self.set_error(t().no_database());
            return;
        };

        match find_page(db, &collection, &query, 0, QUERY_LIMIT, None).await {
            Ok(docs) => {
                let count = docs.len();
                // Store total_count (may be more than returned if limit hit)
                open_state!(self, Query).total_count = count;
                open_state!(self, Query).result_index = 0;
                open_state!(self, Query).results = Some(docs);
                open_state!(self, Query).error = None;
                if count >= QUERY_LIMIT {
                    self.set_status(t().query_found(count, Some(QUERY_LIMIT)));
                } else {
                    self.set_status(t().query_found(count, None));
                }
            }
            Err(e) => {
                open_state!(self, Query).error = Some(t().failed(Op::Query, &e));
            }
        }
    }

    /// Execute query explain (async) - shows query plan
    pub async fn execute_explain_async(&mut self) {
        let query = match open_state!(self, Query).parse_query() {
            Ok(q) => q,
            Err(e) => {
                open_state!(self, Query).error = Some(t().invalid_json(&e));
                return;
            }
        };

        let collection = open_state!(self, Query).collection.clone();

        let Some(db) = &self.db else {
            self.set_error(t().no_database());
            return;
        };

        let args = ExplainParams { collection, query };
        let explained = db
            .call::<Explain>(&args)
            .await
            .and_then(|response| Ok(response.summary()?));
        match explained {
            Ok(plan) => {
                open_state!(self, Query).explain_result = Some(plan);
                open_state!(self, Query).show_explain = true;
                open_state!(self, Query).error = None;
                self.set_status(t().query_plan_loaded());
            }
            Err(e) => {
                open_state!(self, Query).error = Some(t().failed(Op::Explain, &e));
            }
        }
    }

    /// Apply the current query as a filter and close modal
    pub async fn apply_query_as_filter(&mut self) {
        let query = match open_state!(self, Query).parse_query() {
            Ok(q) => q,
            Err(e) => {
                open_state!(self, Query).error = Some(t().invalid_json(&e));
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
        self.reload_documents();
    }

    // === Async Filter ===

    /// Execute the visual filter (async) - max 1000 results
    pub async fn execute_filter_async(&mut self) {
        // Ha van kitöltött field, automatikusan adjuk hozzá keresés előtt
        if !self.filter_state.field_input.is_empty() {
            self.filter_state.add_filter();
        }

        if self.filter_state.filters.is_empty() {
            self.filter_state.error = Some(t().filter_needs_a_condition().to_string());
            return;
        }

        let query = self.filter_state.build_query();
        let collection = match self.current_collection_name() {
            Some(name) => name.to_string(),
            None => {
                self.filter_state.error = Some(t().no_collection_selected().to_string());
                return;
            }
        };

        // Build sort from filter state
        let sort = self.filter_state.build_sort();

        let Some(db) = &self.db else {
            self.filter_state.error = Some(t().no_database().to_string());
            return;
        };

        // First get the total count matching the query
        let total_count = match count_matching(db, &collection, &query).await {
            Ok(c) => c,
            Err(e) => {
                self.filter_state.error = Some(t().failed(Op::Count, &e));
                return;
            }
        };

        if total_count == 0 {
            // 0 eredmény - maradjon nyitva a modal, mutassa a query-t
            self.filter_state.result_count = 0;
            self.filter_state.results = Some(vec![]);
            self.filter_state.error =
                Some(t().filter_no_matches(&serde_json::to_string(&query).unwrap_or_default()));
            return;
        }

        // Load first page of results
        match find_page(db, &collection, &query, 0, self.page_size, sort.as_ref()).await {
            Ok(docs) => {
                self.filter_state.result_count = total_count;
                self.filter_state.results = Some(docs.clone());
                self.filter_state.error = None;

                // Van eredmény - frissítsd a dokumentum listát és zárd be
                self.tasks.cancel(Slot::Documents);
                self.documents = docs;
                self.selected_document = 0;
                self.doc_scroll_offset = 0;
                self.total_docs = total_count; // Use actual total, not loaded count!
                self.active_filter = Some(query.clone()); // Store filter for pagination
                self.active_sort = sort; // Store sort for pagination

                self.set_status(t().match_count(total_count));

                // Close the modal to show results
                self.close_modal();
            }
            Err(e) => {
                self.filter_state.error = Some(t().failed(Op::Filter, &e));
            }
        }
    }

    // === Async Export ===

    /// Execute the export (async) - streaming with batches
    ///
    /// Exports what the document list shows (active filter + sort). The data
    /// goes to a temp file next to the target and is renamed on success, so a
    /// failed export never truncates an existing file.
    pub async fn execute_export_async(&mut self) {
        let Some(state) = self.export_state() else {
            return;
        };
        let file_path = state.file_path.value().to_string();
        let (collection, format, filtered) =
            (state.collection.clone(), state.format, state.filtered);
        let set_error = |app: &mut Self, error: String| {
            if let Some(state) = app.export_state_mut() {
                state.error = Some(error);
            }
        };
        if file_path.trim().is_empty() {
            set_error(self, t().file_path_required().to_string());
            return;
        }

        let Some(tmp_path) = export_temp_path(std::path::Path::new(&file_path)) else {
            set_error(self, t().invalid_path(&file_path));
            return;
        };

        if self.db.is_none() {
            self.set_error(t().no_database());
            return;
        }

        let result = match self
            .write_export_async(&tmp_path, &collection, format)
            .await
        {
            Ok(count) => std::fs::rename(&tmp_path, &file_path)
                .map(|()| count)
                .map_err(|e| t().failed(Op::File, &e)),
            Err(e) => Err(e),
        };

        match result {
            Ok(count) => {
                if let Some(state) = self.export_state_mut() {
                    state.message = Some(t().exported(count, filtered, &file_path));
                    state.error = None;
                }
            }
            Err(e) => {
                let _ = std::fs::remove_file(&tmp_path);
                set_error(self, e);
            }
        }
    }

    /// Write the export into `path` (the temp file); returns the exported
    /// document count. On error the caller removes the file.
    async fn write_export_async(
        &self,
        path: &std::path::Path,
        collection: &str,
        format: crate::modals::export::ExportFormat,
    ) -> Result<usize, String> {
        use crate::modals::export::ExportFormat;
        use std::fs::File;
        use std::io::{BufWriter, Write};

        const BATCH_SIZE: usize = 1000;

        // Same filter + sort as the document list
        let query = self.active_filter.clone().unwrap_or(serde_json::json!({}));
        let sort = self.active_sort.as_ref();

        let Some(db) = &self.db else {
            return Err(t().no_database().to_string());
        };

        let file = File::create(path).map_err(|e| t().failed(Op::File, &e))?;
        let mut writer = BufWriter::new(file);

        let exported = match format {
            ExportFormat::Json => {
                if let Err(e) = writeln!(writer, "[") {
                    return Err(t().failed(Op::Write, &e));
                }

                let mut exported = 0;
                let mut first = true;
                let mut offset = 0;

                loop {
                    let docs =
                        match find_page(db, collection, &query, offset, BATCH_SIZE, sort).await {
                            Ok(d) => d,
                            Err(e) => {
                                return Err(t().failed(Op::Query, &e));
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
                                return Err(t().invalid_json(&e));
                            }
                        };

                        if let Err(e) = write!(writer, "{}{}", prefix, json) {
                            return Err(t().failed(Op::Write, &e));
                        }
                    }

                    exported += docs.len();
                    offset += docs.len();
                }

                if let Err(e) = writeln!(writer, "\n]") {
                    return Err(t().failed(Op::Write, &e));
                }

                exported
            }
            ExportFormat::Csv => {
                let sample = match find_page(db, collection, &query, 0, 100, sort).await {
                    Ok(d) => d,
                    Err(e) => {
                        return Err(t().failed(Op::Query, &e));
                    }
                };

                if sample.is_empty() {
                    return Err(t().nothing_to_export().to_string());
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
                    return Err(t().failed(Op::Write, &e));
                }

                let mut exported = 0;
                let mut offset = 0;

                loop {
                    let docs =
                        match find_page(db, collection, &query, offset, BATCH_SIZE, sort).await {
                            Ok(d) => d,
                            Err(e) => {
                                return Err(t().failed(Op::Query, &e));
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
                            return Err(t().failed(Op::Write, &e));
                        }
                    }

                    exported += docs.len();
                    offset += docs.len();
                }

                exported
            }
        };

        writer.flush().map_err(|e| t().failed(Op::Flush, &e))?;

        Ok(exported)
    }
}

/// Temp file next to the export target (same directory, so the final rename
/// stays on one filesystem)
fn export_temp_path(target: &std::path::Path) -> Option<PathBuf> {
    let name = target.file_name()?.to_string_lossy();
    Some(target.with_file_name(format!(".{}.{}.tmp", name, std::process::id())))
}

/// One page of `find`: `limit` documents after `skip`, optionally sorted
/// The document list's page to load: everything the shown rows depend on
#[derive(Debug, Clone, PartialEq)]
struct DocumentsRequest {
    collection: String,
    filter: Option<Value>,
    sort: Option<Value>,
    skip: usize,
    limit: usize,
}

/// A loaded page of the document list
struct DocumentsPage {
    documents: Vec<Value>,
    /// Matching documents (all of the collection without a filter)
    total: usize,
    /// Without a filter: the collection's count and index names
    details: Option<(usize, Vec<String>)>,
}

/// Load a page of the document list and its counts
async fn fetch_documents(
    db: &DbWrapper,
    request: &DocumentsRequest,
) -> anyhow::Result<DocumentsPage> {
    let query = request.filter.clone().unwrap_or(serde_json::json!({}));
    let documents = find_page(
        db,
        &request.collection,
        &query,
        request.skip,
        request.limit,
        request.sort.as_ref(),
    )
    .await?;
    if let Some(filter) = &request.filter {
        // When filter is active, count matching documents
        let total = count_matching(db, &request.collection, filter).await?;
        Ok(DocumentsPage {
            documents,
            total,
            details: None,
        })
    } else {
        // No filter - get total collection count
        let (doc_count, index_names) = db.load_collection_details(&request.collection).await?;
        Ok(DocumentsPage {
            documents,
            total: doc_count,
            details: Some((doc_count, index_names)),
        })
    }
}

async fn find_page(
    db: &DbWrapper,
    collection: &str,
    query: &Value,
    skip: usize,
    limit: usize,
    sort: Option<&Value>,
) -> anyhow::Result<Vec<Value>> {
    let args = FindParams {
        collection: collection.to_string(),
        query: query.clone(),
        projection: None,
        sort: sort.cloned(),
        limit: Some(limit),
        skip: Some(skip),
        include_total: false,
    };
    Ok(db.call::<Find>(&args).await?.documents)
}

/// Number of documents of `collection` matching `query`
async fn count_matching(db: &DbWrapper, collection: &str, query: &Value) -> anyhow::Result<usize> {
    let args = CountParams {
        collection: collection.to_string(),
        query: query.clone(),
    };
    Ok(db.call::<Count>(&args).await?.count as usize)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{app_with_server, numbered_docs, FakeServer};

    fn doc_ids(app: &App) -> Vec<i64> {
        app.documents
            .iter()
            .map(|d| d["_id"].as_i64().unwrap())
            .collect()
    }

    // === Page size / pagination ===

    /// The page is exactly what the documents pane draws: with the old
    /// maths (inner height, header row not counted) the last fetched row was
    /// never drawn but could be selected
    #[test]
    fn test_every_fetched_row_is_drawn() {
        use ratatui::{backend::TestBackend, Terminal};

        for height in [12u16, 24, 37, 50] {
            let mut app = App::new(Config::default());
            app.update_page_size(height);
            app.total_docs = 1000;
            // Distinct, greppable ids: 1000, 1001, ...
            app.documents = (0..app.page_size as i64)
                .map(|i| serde_json::json!({"_id": 1000 + i, "name": "x"}))
                .collect();

            let mut terminal = Terminal::new(TestBackend::new(120, height)).unwrap();
            terminal.draw(|f| crate::ui::render_ui(f, &app)).unwrap();
            let buffer = terminal.backend().buffer().clone();
            let screen: String = buffer.content().iter().map(|cell| cell.symbol()).collect();

            let last_id = (1000 + app.page_size as i64 - 1).to_string();
            assert!(
                screen.contains(&last_id),
                "height {}: last row {} of the page is not drawn",
                height,
                last_id
            );
        }
    }

    #[test]
    fn test_page_size_never_zero_and_resize_keeps_selection_visible() {
        let mut app = App::new(Config::default());
        for height in 0..=12u16 {
            app.update_page_size(height);
            assert!(app.page_size >= 1);
            let _ = app.get_pagination_info();
        }

        app.update_page_size(40);
        app.total_docs = 100;
        app.selected_document = app.page_size - 1;
        app.update_page_size(20);
        assert!(app.selected_document < app.doc_scroll_offset + app.page_size);
    }

    /// End on the last (partial or scrolled) window is the last page: 100
    /// docs, 9 rows -> offset 91 used to show "11/12"
    #[test]
    fn test_pagination_shows_last_page_at_end() {
        let mut app = App::new(Config::default());
        app.page_size = 9;
        app.total_docs = 100;

        app.doc_scroll_offset = 0;
        assert_eq!(app.get_pagination_info(), (1, 12));
        app.doc_scroll_offset = 9;
        assert_eq!(app.get_pagination_info(), (2, 12));
        app.doc_scroll_offset = 91;
        assert_eq!(app.get_pagination_info(), (12, 12));

        app.total_docs = 5;
        app.doc_scroll_offset = 0;
        assert_eq!(app.get_pagination_info(), (1, 1));
        app.total_docs = 0;
        assert_eq!(app.get_pagination_info(), (0, 0));
    }

    #[tokio::test]
    async fn test_document_window_matches_page_size() {
        let server = FakeServer::new(vec![("docs", numbered_docs(100))]);
        let mut app = app_with_server(&server).await;
        app.page_size = 9;
        app.refresh_documents_async().await.unwrap();
        app.active_pane = Pane::Documents;
        assert_eq!(doc_ids(&app), (0..9).collect::<Vec<_>>());

        // Page 2 loads in the background: until it arrives the rows are
        // page 1's, so nothing is selected (`d`/`e` must not act on them)
        app.page_down_async().await;
        assert!(app.documents_loading());
        assert_eq!(doc_ids(&app), (0..9).collect::<Vec<_>>());
        assert_eq!(app.get_selected_document(), None);
        app.settle().await;
        // Page 2 starts right after the last row of page 1
        assert_eq!(doc_ids(&app), (9..18).collect::<Vec<_>>());
        assert_eq!(app.get_selected_document().unwrap()["_id"], 9);

        // End: the true last document is selected and loaded
        app.go_to_end_async().await;
        app.settle().await;
        assert_eq!(doc_ids(&app), (91..100).collect::<Vec<_>>());
        assert_eq!(app.get_selected_document().unwrap()["_id"], 99);
        assert_eq!(app.get_pagination_info(), (12, 12));
    }

    // === Collection identity ===

    /// The server lists collections in HashMap order: the list is sorted and
    /// the selection follows the collection NAME across a refresh
    #[tokio::test]
    async fn test_refresh_keeps_selected_collection_by_name() {
        let server = FakeServer::new(vec![
            ("zebra", numbered_docs(1)),
            ("apple", numbered_docs(2)),
            ("mango", numbered_docs(3)),
        ]);
        let mut app = app_with_server(&server).await;
        let names: Vec<&str> = app.collections.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["apple", "mango", "zebra"]);

        app.select_down_async().await;
        assert_eq!(app.current_collection_name(), Some("mango"));
        app.active_filter = Some(serde_json::json!({"even": true}));

        // Another client creates collections; the server order changes too
        {
            let mut collections = server.collections.lock().unwrap();
            collections.push(("banana".to_string(), numbered_docs(4)));
            collections.push(("aaa".to_string(), numbered_docs(5)));
            collections.reverse();
        }
        app.refresh_collections_async().await.unwrap();
        assert_eq!(app.current_collection_name(), Some("mango"));
        // Same collection: its filter stays, the documents are its documents
        assert!(app.active_filter.is_some());
        assert_eq!(app.total_docs, 2);
    }

    /// The selected collection disappeared: clamped index, and the filter of
    /// the old collection must not be applied to the new one
    #[tokio::test]
    async fn test_refresh_after_selected_collection_vanished() {
        let server = FakeServer::new(vec![
            ("a", numbered_docs(4)),
            ("b", numbered_docs(6)),
            ("c", numbered_docs(8)),
        ]);
        let mut app = app_with_server(&server).await;
        app.go_to_end_async().await;
        assert_eq!(app.current_collection_name(), Some("c"));
        app.active_filter = Some(serde_json::json!({"even": true}));
        app.active_sort = Some(serde_json::json!({"n": -1}));

        server.collections.lock().unwrap().retain(|(n, _)| n != "c");
        app.refresh_collections_async().await.unwrap();

        assert_eq!(app.current_collection_name(), Some("b"));
        assert_eq!(app.active_filter, None);
        assert_eq!(app.active_sort, None);
        assert_eq!(app.total_docs, 6);
        assert_eq!(app.documents.len(), 6);
    }

    /// Every way of switching collection drops the filter and sort
    #[tokio::test]
    async fn test_every_collection_switch_clears_filter_and_sort() {
        let names: Vec<String> = (0..25).map(|i| format!("c{:02}", i)).collect();
        let server = FakeServer::new(
            names
                .iter()
                .map(|n| (n.as_str(), numbered_docs(3)))
                .collect(),
        );
        let mut app = app_with_server(&server).await;

        async fn check(app: &mut App, step: &str, expected: &str) {
            app.active_filter = Some(serde_json::json!({"even": true}));
            app.active_sort = Some(serde_json::json!({"n": -1}));
            app.doc_scroll_offset = 1;
            app.selected_document = 2;
            match step {
                "down" => app.select_down_async().await,
                "up" => app.select_up_async().await,
                "pgdn" => app.page_down_async().await,
                "pgup" => app.page_up_async().await,
                "end" => app.go_to_end_async().await,
                "home" => app.go_to_start_async().await,
                _ => {
                    app.search.results = vec![SearchResult {
                        name: expected.to_string(),
                        doc_count: 3,
                    }];
                    app.search.selected_result = 0;
                    app.goto_search_result_async().await;
                    app.active_pane = Pane::Collections;
                }
            }
            assert_eq!(app.current_collection_name(), Some(expected), "{}", step);
            assert_eq!(app.active_filter, None, "{}", step);
            assert_eq!(app.active_sort, None, "{}", step);
            assert_eq!((app.doc_scroll_offset, app.selected_document), (0, 0));
            assert_eq!(app.total_docs, 3, "{}", step);
        }

        check(&mut app, "down", "c01").await;
        check(&mut app, "pgdn", "c11").await;
        check(&mut app, "up", "c10").await;
        check(&mut app, "pgup", "c00").await;
        check(&mut app, "end", "c24").await;
        check(&mut app, "home", "c00").await;
        check(&mut app, "search", "c07").await;
    }

    /// Esc while a page loads stops it; the old page's rows are not left
    /// under the new position
    #[tokio::test]
    async fn test_esc_stops_a_document_load() {
        let server = FakeServer::new(vec![("docs", numbered_docs(100))]);
        let mut app = app_with_server(&server).await;
        app.page_size = 9;
        app.refresh_documents_async().await.unwrap();
        app.active_pane = Pane::Documents;

        app.page_down_async().await;
        assert!(app.tasks.is_busy());
        let esc = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Esc,
            crossterm::event::KeyModifiers::NONE,
        );
        crate::handlers::handle_key(&mut app, esc).await;
        app.settle().await;

        assert!(!app.tasks.is_busy());
        assert!(app.documents.is_empty());
        assert_eq!(app.get_selected_document(), None);
        assert_eq!(app.status_message.as_deref(), Some(t().tasks_cancelled()));
    }

    /// A page that arrives after the view changed without a new load (here:
    /// a filter set directly) is not shown
    #[tokio::test]
    async fn test_a_page_for_another_view_is_dropped() {
        let server = FakeServer::new(vec![("docs", numbered_docs(100))]);
        let mut app = app_with_server(&server).await;
        app.page_size = 9;
        app.refresh_documents_async().await.unwrap();

        app.doc_scroll_offset = 9;
        app.reload_documents();
        app.active_filter = Some(serde_json::json!({"_id": 1}));
        app.settle().await;

        assert_eq!(doc_ids(&app), (0..9).collect::<Vec<_>>());
    }

    // === Failed refresh ===

    /// A failed document refresh must not leave the previous collection's
    /// documents on screen (`d`/`e` would act on them under the new name)
    #[tokio::test]
    async fn test_failed_refresh_clears_documents_and_reports() {
        let server = FakeServer::new(vec![("a", numbered_docs(4)), ("b", numbered_docs(6))]);
        let mut app = app_with_server(&server).await;
        assert_eq!(app.documents.len(), 4);
        app.selected_document = 2;

        *server.failing_find.lock().unwrap() = Some("b".to_string());
        app.select_down_async().await;
        app.settle().await;

        assert_eq!(app.current_collection_name(), Some("b"));
        assert!(app.documents.is_empty());
        assert_eq!(app.get_selected_document(), None);
        assert_eq!(
            (app.selected_document, app.doc_scroll_offset, app.total_docs),
            (0, 0, 0)
        );
        assert!(app
            .error_message
            .as_deref()
            .unwrap()
            .contains("find failed"));
    }

    #[tokio::test]
    async fn test_delete_collection_clears_active_filter() {
        let server = FakeServer::new(vec![("a", numbered_docs(4)), ("b", numbered_docs(6))]);
        let mut app = app_with_server(&server).await;
        app.select_down_async().await;
        app.active_filter = Some(serde_json::json!({"even": true}));
        app.active_sort = Some(serde_json::json!({"n": -1}));

        app.do_delete_collection_async("b").await;

        assert_eq!(app.current_collection_name(), Some("a"));
        assert_eq!(app.active_filter, None);
        assert_eq!(app.active_sort, None);
        assert_eq!(app.total_docs, 4);
    }

    #[test]
    fn test_delete_document_confirm_names_the_collection() {
        let mut app = App::new(Config::default());
        app.collections = vec![CollectionInfo::unloaded("invoices".to_string())];
        app.documents = vec![serde_json::json!({"_id": 7})];
        app.open_delete_confirm();
        assert_eq!(app.modal(), Some(Modal::Confirm));
        assert!(app.confirm_state().unwrap().message.contains("invoices"));
    }

    // === Export ===

    fn export_target(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("tuti-tui-export-{}-{}", name, std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("out.v1").join("..").join("docs.json")
    }

    fn files_in(dir: &std::path::Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    /// The export is what the document list shows: filtered and sorted
    #[tokio::test]
    async fn test_export_uses_active_filter_and_sort() {
        let server = FakeServer::new(vec![("docs", numbered_docs(10))]);
        let mut app = app_with_server(&server).await;
        app.active_filter = Some(serde_json::json!({"even": true}));
        app.active_sort = Some(serde_json::json!({"n": -1}));
        app.refresh_documents_async().await.unwrap();

        app.open_export_modal();
        assert_eq!(app.export_state().unwrap().doc_count, 5);
        assert!(app.export_state().unwrap().filtered);

        let target = export_target("filtered");
        let dir = target
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .to_path_buf();
        std::fs::create_dir_all(dir.join("out.v1")).unwrap();
        app.export_state_mut()
            .unwrap()
            .file_path
            .set_value(target.to_string_lossy().into_owned());
        app.execute_export_async().await;
        assert_eq!(app.export_state().unwrap().error, None);
        assert!(app
            .export_state()
            .unwrap()
            .message
            .as_deref()
            .unwrap()
            .starts_with("5 "));

        let exported: Vec<Value> =
            serde_json::from_str(&std::fs::read_to_string(dir.join("docs.json")).unwrap()).unwrap();
        let ids: Vec<i64> = exported
            .iter()
            .map(|d| d["_id"].as_i64().unwrap())
            .collect();
        assert_eq!(ids, [8, 6, 4, 2, 0]);
        // No temp file left behind
        assert_eq!(files_in(&dir), ["docs.json", "out.v1"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Without a filter the whole collection is exported and the modal says so
    #[tokio::test]
    async fn test_export_without_filter_is_unfiltered() {
        let server = FakeServer::new(vec![("docs", numbered_docs(10))]);
        let mut app = app_with_server(&server).await;
        app.open_export_modal();
        assert_eq!(app.export_state().unwrap().doc_count, 10);
        assert!(!app.export_state().unwrap().filtered);
    }

    /// A failed export reports the error, keeps the existing target file
    /// untouched and leaves no temp file
    #[tokio::test]
    async fn test_failed_export_keeps_existing_file() {
        let server = FakeServer::new(vec![("docs", numbered_docs(10))]);
        let mut app = app_with_server(&server).await;
        app.open_export_modal();

        let dir = std::env::temp_dir().join(format!("tuti-tui-export-fail-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let target = dir.join("docs.json");
        std::fs::write(&target, "precious").unwrap();
        app.export_state_mut()
            .unwrap()
            .file_path
            .set_value(target.to_string_lossy().into_owned());

        *server.failing_find.lock().unwrap() = Some("docs".to_string());
        app.execute_export_async().await;

        assert!(app
            .export_state()
            .unwrap()
            .error
            .as_deref()
            .unwrap()
            .contains("find failed"));
        assert_eq!(app.export_state().unwrap().message, None);
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "precious");
        assert_eq!(files_in(&dir), ["docs.json"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_export_temp_path_is_next_to_target() {
        let tmp = export_temp_path(std::path::Path::new("/data/v1.2/users.json")).unwrap();
        assert_eq!(tmp.parent(), Some(std::path::Path::new("/data/v1.2")));
        assert_ne!(tmp.file_name().unwrap(), "users.json");
        assert_eq!(export_temp_path(std::path::Path::new("/data/..")), None);
    }

    #[test]
    fn test_set_status_clears_stale_error() {
        let mut app = App::new(Config::default());
        app.set_error("regi hiba");
        app.set_status("Dokumentum törölve");
        assert_eq!(app.error_message, None);
        assert_eq!(app.status_message.as_deref(), Some("Dokumentum törölve"));

        // A later error is shown again (and `e` opens its details)
        app.set_error("uj hiba");
        assert_eq!(app.error_message.as_deref(), Some("uj hiba"));
    }

    #[test]
    fn test_drop_index_needs_confirm_and_returns_to_index_modal() {
        let mut app = App::new(Config::default());
        app.open_modal = Some(crate::app::OpenModal::Index(IndexState::new(
            "users".to_string(),
            vec!["users_id".to_string(), "users_age".to_string()],
        )));
        app.index_state_mut().unwrap().selected_index = 1;

        app.open_drop_index_confirm();
        assert_eq!(app.modal(), Some(Modal::Confirm));
        assert_eq!(app.confirm_state().unwrap().selected, ConfirmOption::Cancel);
        assert!(app.confirm_state().unwrap().message.contains("users_age"));
        assert!(app.confirm_state().unwrap().message.contains("users"));
        assert!(matches!(
            &app.confirm_state().unwrap().on_confirm,
            ConfirmAction::DropIndex { collection, index_name }
                if collection == "users" && index_name == "users_age"
        ));

        // Cancel: back in the index modal, state intact
        app.close_confirm();
        assert_eq!(app.modal(), Some(Modal::Index));
        assert_eq!(app.index_state_mut().unwrap().selected_index, 1);
        assert_eq!(app.index_state_mut().unwrap().indexes.len(), 2);
    }

    #[test]
    fn test_primary_id_index_drop_is_refused() {
        let mut app = App::new(Config::default());
        app.open_modal = Some(crate::app::OpenModal::Index(IndexState::new(
            "users".to_string(),
            vec!["users_id".to_string()],
        )));

        app.open_drop_index_confirm();
        assert_eq!(app.modal(), Some(Modal::Index));
        assert!(app.index_state_mut().unwrap().error.is_some());
    }

    #[test]
    fn test_rollback_confirm_names_script_and_version() {
        let mut app = App::new(Config::default());
        app.open_script_modal();
        app.script_state_mut().unwrap().name.set_value("cleanup");
        app.script_state_mut().unwrap().versions = vec![ScriptVersion {
            script_name: "greet".to_string(),
            version: 3,
            code: String::new(),
            description: None,
            tags: vec![],
            dependencies: vec![],
            created_at: String::new(),
        }];

        app.open_rollback_script_confirm();
        assert_eq!(app.modal(), Some(Modal::Confirm));
        assert!(app.confirm_state().unwrap().message.contains("cleanup"));
        assert!(app.confirm_state().unwrap().message.contains("v3"));
        assert!(matches!(
            &app.confirm_state().unwrap().on_confirm,
            ConfirmAction::RollbackScript { name, version } if name == "cleanup" && *version == 3
        ));

        app.close_confirm();
        assert_eq!(app.modal(), Some(Modal::Script));
    }

    #[test]
    fn test_rag_destructive_actions_need_confirm_and_return_to_rag() {
        let mut app = App::new(Config::default());
        app.open_modal = Some(OpenModal::Rag(RagState::new("kb".to_string())));
        app.rag_state_mut().unwrap().jobs = vec![ironbase_mcp_types::responses::JobInfo {
            id: "job-1".to_string(),
            job_type: ironbase_mcp_types::responses::JobType::Compact,
            status: ironbase_mcp_types::responses::JobStatus::Pending,
            created_at_secs: 0,
            updated_at_secs: 0,
            items_processed: 0,
            items_total: None,
        }];

        app.open_disable_auto_embed_confirm();
        assert_eq!(app.modal(), Some(Modal::Confirm));
        assert!(app.confirm_state().unwrap().message.contains("kb"));
        app.close_confirm();
        assert_eq!(app.modal(), Some(Modal::Rag));

        app.open_clear_embed_cache_confirm();
        assert_eq!(app.modal(), Some(Modal::Confirm));
        assert!(matches!(
            app.confirm_state().unwrap().on_confirm,
            ConfirmAction::ClearEmbedCache
        ));
        app.close_confirm();
        assert_eq!(app.modal(), Some(Modal::Rag));

        app.open_cancel_embed_job_confirm();
        assert_eq!(app.modal(), Some(Modal::Confirm));
        assert!(app.confirm_state().unwrap().message.contains("job-1"));
        app.close_confirm();
        assert_eq!(app.modal(), Some(Modal::Rag));
    }

    #[test]
    fn test_document_delete_confirm_closes_to_panes() {
        let mut app = App::new(Config::default());
        app.open_confirm(ConfirmState::delete_collection("users".to_string(), 0));
        app.close_confirm();
        assert_eq!(app.modal(), None);
    }

    /// A modal's state lives with the open modal: closing drops it (reopening
    /// starts empty) and a late answer for a closed modal changes nothing
    /// A confirm dialog keeps the modal it was opened from and gives it back
    /// as it was (it used to reopen the modal by kind only)
    #[test]
    fn test_confirm_gives_back_the_modal_it_was_opened_from() {
        let mut app = App::new(Config::default());
        let mut state = ExportState::new("users".to_string(), 3);
        state.file_path.insert_str("-draft");
        app.open_modal = Some(OpenModal::Export(state));

        app.open_confirm(ConfirmState::clear_embed_cache());
        assert_eq!(app.modal(), Some(Modal::Confirm));
        assert!(app.export_state().is_none());
        app.close_confirm();
        assert_eq!(
            app.export_state().unwrap().file_path.value(),
            "users.json-draft"
        );

        // A confirm with nothing under it closes to the panes
        app.close_modal();
        app.open_confirm(ConfirmState::clear_embed_cache());
        app.close_confirm();
        assert_eq!(app.modal(), None);
    }

    /// Closing a confirm dialog leaves any other open modal alone, and a
    /// modal opened by kind has the state its keys need
    #[test]
    fn test_modal_by_kind_has_state_and_close_confirm_keeps_others() {
        let mut app = App::new(Config::default());
        app.set_modal(Modal::Index);
        assert!(app.index_state().is_some());
        app.close_confirm();
        assert_eq!(app.modal(), Some(Modal::Index));

        app.set_modal(Modal::Confirm);
        assert!(app.confirm_state().is_some());
        app.close_confirm();
        assert_eq!(app.modal(), None);
    }

    #[test]
    fn test_modal_state_lives_with_the_open_modal() {
        let mut app = App::new(Config::default());
        app.open_new_collection();
        if let Some(OpenModal::NewCollection(state)) = app.open_modal_mut() {
            state.name.insert_str("draft");
        }
        app.close_modal();
        app.open_new_collection();
        let Some(OpenModal::NewCollection(state)) = app.open_modal() else {
            panic!("not open");
        };
        assert_eq!(state.name.value(), "");

        app.close_modal();
        app.set_server_info_error("late".to_string());
        app.set_update_error("late".to_string());
        assert_eq!(app.modal(), None);
    }
}
