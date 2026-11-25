//! UI language: every text the user reads comes from [`t()`], a
//! [`Messages`] implementation per language. A text is a trait method, so a
//! language that misses one does not compile, and a text with values in it
//! takes them as typed arguments.
//!
//! The language is process-wide (like a locale): it is set once at startup
//! from the config or the system, and read wherever text is drawn, without
//! threading it through every render function.

mod en;
mod hu;

use crate::keymap::{Action, Section};
use crate::state::filter::{FilterOperator, SortDirection};
use crate::state::query::QueryTemplate;
use serde::{Deserialize, Serialize};
use std::fmt::Display;
use std::sync::atomic::{AtomicU8, Ordering};

pub use en::En;
pub use hu::Hu;

/// A UI language (`language = "hu"` / `"en"` in the config)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Language {
    Hu,
    En,
}

impl Language {
    /// The language of a locale string such as `hu_HU.UTF-8`; English for
    /// anything but Hungarian
    pub fn from_locale(locale: &str) -> Self {
        if locale.to_ascii_lowercase().starts_with("hu") {
            Language::Hu
        } else {
            Language::En
        }
    }

    /// The language of a config value (any letter case); `None` for an
    /// unknown one
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "hu" => Some(Language::Hu),
            "en" => Some(Language::En),
            _ => None,
        }
    }

    /// Serde: an unknown language is unset (with a warning in the log), so
    /// one bad value does not make the whole config unreadable
    pub fn deserialize_lenient<'de, D>(deserializer: D) -> Result<Option<Self>, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = Option::<String>::deserialize(deserializer)?;
        Ok(value.and_then(|value| {
            let language = Self::parse(&value);
            if language.is_none() {
                tracing::warn!(value = %value, "Unknown UI language in the config, using the system's");
            }
            language
        }))
    }

    /// The system's language: the first set of `LC_ALL`, `LC_MESSAGES`,
    /// `LANG` (the POSIX order); English when none is set
    pub fn system() -> Self {
        ["LC_ALL", "LC_MESSAGES", "LANG"]
            .iter()
            .filter_map(|name| std::env::var(name).ok())
            .find(|value| !value.is_empty())
            .map(|locale| Self::from_locale(&locale))
            .unwrap_or(Language::En)
    }
}

/// Hungarian until `set_language` (the texts the tests check are Hungarian)
static CURRENT: AtomicU8 = AtomicU8::new(Language::Hu as u8);

/// Switch the UI language (at startup)
pub fn set_language(language: Language) {
    CURRENT.store(language as u8, Ordering::Relaxed);
}

pub fn language() -> Language {
    if CURRENT.load(Ordering::Relaxed) == Language::En as u8 {
        Language::En
    } else {
        Language::Hu
    }
}

/// The texts of the current language
pub fn t() -> &'static dyn Messages {
    match language() {
        Language::Hu => &Hu,
        Language::En => &En,
    }
}

/// Every text of the UI, one method each. Every method returns the text in
/// its own language; the forms quoted in the docs below are the English ones,
/// as examples.
pub trait Messages: Sync + sealed::Sealed {
    // === Header ===
    /// `IronBase v{version} | {db} ({collections} coll, {docs} docs)`
    fn header_db(&self, version: &str, db: &str, collections: usize, docs: usize) -> String;
    fn pane_collections(&self) -> &'static str;
    fn pane_documents(&self) -> &'static str;
    fn pane_detail(&self) -> &'static str;

    // === Command bar ===
    fn cmd_panel(&self) -> &'static str;
    fn cmd_navigate(&self) -> &'static str;
    fn cmd_select(&self) -> &'static str;
    fn cmd_page(&self) -> &'static str;
    fn cmd_scroll(&self) -> &'static str;
    fn cmd_refresh(&self) -> &'static str;
    fn cmd_filter(&self) -> &'static str;
    fn cmd_search(&self) -> &'static str;
    fn cmd_actions(&self) -> &'static str;
    fn cmd_edit(&self) -> &'static str;
    fn cmd_delete(&self) -> &'static str;
    fn cmd_copy(&self) -> &'static str;
    fn cmd_help(&self) -> &'static str;
    fn cmd_quit(&self) -> &'static str;
    /// The error shown in the command bar
    fn bar_error(&self, error: &str) -> String;
    fn bar_error_details(&self) -> &'static str;

    // === Panes ===
    fn collections_title(&self) -> &'static str;
    fn no_collections(&self) -> &'static str;
    fn indexes_title(&self) -> &'static str;
    fn no_indexes(&self) -> &'static str;
    fn documents_title(&self, page: usize, pages: usize, filtered: bool) -> String;
    fn no_documents(&self) -> &'static str;
    fn select_collection(&self) -> &'static str;
    /// The detail pane title; `matches` is (current, total) while searching
    fn detail_title(&self, matches: Option<(usize, usize)>) -> String;
    fn select_document(&self) -> &'static str;

    // === Status and errors ===
    fn no_database(&self) -> &'static str;
    fn no_collection_selected(&self) -> &'static str;
    fn no_document_selected(&self) -> &'static str;
    fn no_script_selected(&self) -> &'static str;
    fn no_version_selected(&self) -> &'static str;
    /// The failure of an operation, e.g. `Search failed: {error}`
    fn failed(&self, op: Op, error: &dyn Display) -> String;
    /// A failure with no operation to name, e.g. `Error: {error}`
    fn error(&self, error: &dyn Display) -> String;
    fn clipboard_unavailable(&self, error: &dyn Display) -> String;
    fn invalid_json(&self, error: &dyn Display) -> String;
    fn invalid_json_params(&self, error: &dyn Display) -> String;
    fn name_required(&self) -> &'static str;
    fn field_name_required(&self) -> &'static str;
    fn file_path_required(&self) -> &'static str;
    fn invalid_path(&self, path: &str) -> String;
    fn theme_changed(&self, theme: &str) -> String;
    fn copied(&self, preview: &str) -> String;

    // Search and filter
    fn no_matches(&self) -> &'static str;
    fn match_count(&self, count: usize) -> String;
    /// The match count with the keys that step through them
    fn match_count_with_hint(&self, count: usize) -> String;
    fn match_position(&self, current: usize, total: usize) -> String;
    fn filter_needs_a_condition(&self) -> &'static str;
    fn filter_no_matches(&self, query: &str) -> String;

    // Collections and documents
    fn collection_exists(&self) -> &'static str;
    fn collection_created(&self, name: &str) -> String;
    fn collection_dropped(&self, name: &str) -> String;
    fn document_deleted(&self) -> &'static str;
    fn document_not_found(&self) -> &'static str;
    fn document_unchanged(&self) -> &'static str;
    fn document_updated(&self) -> &'static str;
    /// The update is stored, only re-embedding it failed
    fn document_updated_vector_stale(&self, error: &dyn Display) -> String;
    fn document_inserted(&self, id: &dyn Display) -> String;
    fn document_must_be_object(&self) -> &'static str;
    fn document_id_missing(&self) -> &'static str;
    /// `limit`: the result count reached the query limit
    fn query_found(&self, count: usize, limit: Option<usize>) -> String;
    fn query_plan_loaded(&self) -> &'static str;
    fn exported(&self, count: usize, filtered: bool, path: &str) -> String;
    fn nothing_to_export(&self) -> &'static str;

    // Indexes
    fn primary_index_not_droppable(&self, index: &str) -> String;
    /// `fields` joined, `flags` such as ` [unique]` (or empty)
    fn index_created(&self, fields: &str, compound: bool, flags: &str, name: &str) -> String;
    fn index_dropped(&self, name: &str) -> String;
    fn analyzing_indexes(&self) -> &'static str;
    fn index_statistics_refreshed(&self) -> &'static str;
    fn vector_input_empty(&self) -> &'static str;
    fn vector_empty(&self) -> &'static str;
    fn invalid_vector(&self, error: &dyn Display) -> String;
    fn invalid_filter(&self, error: &dyn Display) -> String;

    // RAG and embedding
    fn auto_embed_disabled(&self) -> &'static str;
    /// `search` summary: what the server left out of the results
    fn search_summary(
        &self,
        found: usize,
        over_budget: usize,
        without_body: usize,
        trimmed: bool,
    ) -> String;

    // Scripts
    fn script_name_required(&self) -> &'static str;
    fn script_saved(&self, version: u32) -> String;
    fn script_deleted(&self, name: &str) -> String;
    fn script_rolled_back(&self, version: u32) -> String;
    /// `search` answered in another shape than the one asked for
    fn unexpected_search_response(&self) -> &'static str;

    // === Modals (shared keys and labels, then per modal) ===
    fn key_save(&self) -> &'static str;
    fn key_cancel(&self) -> &'static str;
    fn key_close(&self) -> &'static str;
    fn key_create(&self) -> &'static str;
    fn key_search(&self) -> &'static str;
    fn key_search_or_jump(&self) -> &'static str;
    fn key_format(&self) -> &'static str;
    fn key_export(&self) -> &'static str;
    fn key_switch_mode(&self) -> &'static str;
    fn key_switch_field(&self) -> &'static str;
    fn key_navigate(&self) -> &'static str;
    fn key_indent(&self) -> &'static str;
    fn label_error(&self) -> &'static str;
    fn label_name(&self) -> &'static str;
    fn label_file(&self) -> &'static str;
    fn label_format(&self) -> &'static str;
    fn label_path(&self) -> &'static str;
    fn label_mode(&self) -> &'static str;
    fn label_field(&self) -> &'static str;
    fn collection_label(&self, name: &str) -> String;
    fn error_details_title(&self) -> &'static str;
    fn error_scroll_hint(&self, current: usize, total: usize) -> String;
    fn esc_to_close(&self) -> &'static str;
    fn new_collection_title(&self) -> &'static str;
    fn collection_name_hint(&self) -> &'static str;
    fn edit_document_title(&self) -> &'static str;
    fn new_document_title(&self) -> &'static str;
    fn cursor_position(&self, line: usize, column: usize) -> String;
    fn update_check_title(&self) -> &'static str;
    fn checking_for_updates(&self) -> &'static str;
    fn current_version(&self) -> &'static str;
    fn latest_version(&self) -> &'static str;
    fn update_available(&self) -> &'static str;
    fn update_instructions(&self) -> &'static str;
    fn whats_new(&self) -> &'static str;
    fn up_to_date(&self) -> &'static str;
    fn export_title(&self) -> &'static str;
    fn export_collection_info(&self, collection: &str, count: usize, filtered: bool) -> String;
    fn press_enter_to_export(&self) -> &'static str;
    fn search_collections_title(&self) -> &'static str;
    fn search_document_title(&self) -> &'static str;
    fn type_collection_name(&self) -> &'static str;
    fn type_search_text(&self) -> &'static str;
    fn collection_with_count(&self, name: &str, documents: usize) -> String;
    fn start_typing_to_search(&self) -> &'static str;
    fn document_match_help(&self, current: usize, total: usize) -> String;
    fn open_database_title(&self) -> &'static str;
    fn create_database_title(&self) -> &'static str;
    fn mode_open(&self) -> &'static str;
    fn mode_create(&self) -> &'static str;
    fn open_database_info(&self) -> &'static str;
    fn create_database_info(&self) -> &'static str;
    fn connecting(&self) -> &'static str;
    fn open_database_tip(&self) -> &'static str;
    fn create_database_tip(&self) -> &'static str;
    fn fulltext_title(&self) -> &'static str;
    fn no_fulltext_index(&self) -> &'static str;
    fn results_title(&self, count: usize, more: bool) -> String;
    fn searching(&self) -> &'static str;
    fn confirm_title(&self) -> &'static str;
    fn confirm_message(&self) -> &'static str;
    fn yes(&self) -> &'static str;
    fn confirm_delete_document_title(&self) -> &'static str;
    fn confirm_delete_document(&self, collection: &str, preview: &str) -> String;
    fn confirm_drop_collection_title(&self) -> &'static str;
    fn confirm_drop_collection(&self, name: &str, documents: usize) -> String;
    fn confirm_drop_index_title(&self) -> &'static str;
    fn confirm_drop_index(&self, index: &str, collection: &str) -> String;
    fn confirm_drop_vector_index_title(&self) -> &'static str;
    fn confirm_drop_vector_index(&self, index: &str, collection: &str) -> String;
    fn confirm_rollback_title(&self) -> &'static str;
    fn confirm_rollback(&self, name: &str, version: u32) -> String;
    fn rollback_button(&self) -> &'static str;
    fn confirm_disable_auto_embed_title(&self) -> &'static str;
    fn confirm_disable_auto_embed(&self, collection: &str) -> String;
    fn disable_button(&self) -> &'static str;
    fn confirm_clear_cache_title(&self) -> &'static str;
    fn confirm_clear_cache(&self) -> &'static str;
    fn clear_button(&self) -> &'static str;
    fn confirm_cancel_job_title(&self) -> &'static str;
    fn confirm_cancel_job(&self, job_id: &str) -> String;
    fn cancel_job_button(&self) -> &'static str;

    // Filter, query and server info modals
    fn filter_title(&self) -> &'static str;
    fn key_choose_field(&self) -> &'static str;
    fn key_next(&self) -> &'static str;
    fn field_title(&self, count: usize) -> String;
    fn field_placeholder_examples(&self) -> &'static str;
    fn field_placeholder_pick(&self) -> &'static str;
    fn label_operator(&self) -> &'static str;
    fn label_value(&self) -> &'static str;
    fn value_placeholder(&self) -> &'static str;
    fn hint_type_field(&self) -> &'static str;
    fn hint_field_next(&self) -> &'static str;
    fn hint_operator(&self) -> &'static str;
    fn hint_add_exists(&self) -> &'static str;
    fn hint_type_value(&self) -> &'static str;
    fn hint_add_condition(&self) -> &'static str;
    fn hint_filters(&self) -> &'static str;
    fn hint_sort(&self) -> &'static str;
    fn active_filters_title(&self, count: usize) -> String;
    fn no_active_filters(&self) -> &'static str;
    fn label_sort(&self) -> &'static str;
    fn no_sort(&self) -> &'static str;
    fn press_ctrl_s_to_search(&self) -> &'static str;
    fn add_filters(&self) -> &'static str;
    fn query_title(&self) -> &'static str;
    fn key_run(&self) -> &'static str;
    fn key_plan(&self) -> &'static str;
    fn key_templates(&self) -> &'static str;
    fn query_results_header(&self, count: usize, total: usize) -> String;
    fn label_result(&self) -> &'static str;
    fn query_prompt(&self) -> &'static str;
    fn query_status(&self, line: usize, column: usize, collection: &str, plan: bool) -> String;
    fn query_plan_title(&self) -> &'static str;
    fn label_plan(&self) -> &'static str;
    /// The same word in both languages
    fn label_index(&self) -> &'static str;
    fn label_estimated_rows(&self) -> &'static str;
    fn label_stage(&self) -> &'static str;
    fn label_cost(&self) -> &'static str;
    fn label_reason(&self) -> &'static str;
    fn no_query_plan(&self) -> &'static str;
    fn query_plan_toggle_hint(&self) -> &'static str;
    fn server_info_title(&self) -> &'static str;
    fn loading(&self) -> &'static str;
    fn database_info(&self) -> &'static str;
    fn label_server(&self) -> &'static str;
    fn label_api_key(&self) -> &'static str;
    fn api_key_required(&self) -> &'static str;
    fn api_key_not_required(&self) -> &'static str;
    fn label_database(&self) -> &'static str;
    fn label_file_size(&self) -> &'static str;
    fn label_collection_count(&self) -> &'static str;
    fn label_total_documents(&self) -> &'static str;
    fn label_total_indexes(&self) -> &'static str;
    fn label_of_which_vector(&self) -> &'static str;
    fn vector_index_summary(&self, indexes: usize, vectors: usize) -> String;
    fn label_collections(&self) -> &'static str;
    fn available_tools(&self, count: usize) -> String;
    fn available_prompts(&self, count: usize) -> String;
    fn no_description(&self) -> &'static str;
    fn protocol_version(&self, version: &str) -> String;
    fn filter_operator(&self, operator: FilterOperator) -> &'static str;
    fn sort_direction(&self, direction: SortDirection) -> &'static str;
    fn query_template(&self, template: QueryTemplate) -> &'static str;
    fn tool_category(&self, category: ToolCategory) -> &'static str;

    // API key, listener, vector and index modals
    fn no(&self) -> &'static str;
    fn key_new(&self) -> &'static str;
    fn key_tabs(&self) -> &'static str;
    fn key_next_field(&self) -> &'static str;
    fn label_status(&self) -> &'static str;
    fn label_field_name(&self) -> &'static str;
    fn status_active(&self) -> &'static str;
    fn api_keys_title(&self) -> &'static str;
    fn key_revoke(&self) -> &'static str;
    fn admin_needs_localhost(&self) -> &'static str;
    fn admin_needs_key(&self) -> &'static str;
    fn new_key_label(&self) -> &'static str;
    fn new_key_warning(&self) -> &'static str;
    fn no_api_keys(&self) -> &'static str;
    fn label_key(&self) -> &'static str;
    fn status_revoked(&self) -> &'static str;
    fn enter_new_key_name(&self) -> &'static str;
    fn confirm_revoke_key(&self, name: &str) -> String;
    fn confirm_delete_key(&self, name: &str) -> String;
    fn listeners_title(&self) -> &'static str;
    fn key_enable(&self) -> &'static str;
    fn key_disable(&self) -> &'static str;
    fn restart_needed(&self) -> &'static str;
    fn no_listeners(&self) -> &'static str;
    fn label_bind(&self) -> &'static str;
    fn status_disabled(&self) -> &'static str;
    fn enter_listener_details(&self) -> &'static str;
    fn label_bind_address(&self) -> &'static str;
    fn checkbox_enabled(&self) -> &'static str;
    fn checkbox_disabled(&self) -> &'static str;
    fn space_to_toggle(&self) -> &'static str;
    fn confirm_delete_listener(&self, id: &str) -> String;
    fn confirm_enable_listener(&self, id: &str) -> String;
    fn confirm_disable_listener(&self, id: &str) -> String;
    fn listener_read_only(&self) -> &'static str;
    fn vector_search_title(&self) -> &'static str;
    fn tab_create_index(&self) -> &'static str;
    fn tab_list_indexes(&self) -> &'static str;
    fn label_k_limit(&self) -> &'static str;
    fn label_vector_json(&self) -> &'static str;
    fn label_filter_optional(&self) -> &'static str;
    fn vector_results_title(&self, count: usize, loading: bool) -> String;
    fn no_vector_results(&self) -> &'static str;
    fn key_toggle_metric(&self) -> &'static str;
    fn label_dimension(&self) -> &'static str;
    fn label_distance_metric(&self) -> &'static str;
    fn metric_cosine(&self) -> &'static str;
    fn metric_euclidean(&self) -> &'static str;
    fn metric_dot_product(&self) -> &'static str;
    fn dimension_tip(&self) -> &'static str;
    fn vector_indexes_title(&self, count: usize) -> String;
    fn no_vector_indexes(&self) -> &'static str;
    fn vector_index_row(
        &self,
        name: &str,
        field: &str,
        dimension: usize,
        metric: &str,
        vectors: usize,
    ) -> String;
    fn index_title(&self) -> &'static str;
    fn key_analyze(&self) -> &'static str;
    fn existing_indexes_title(&self, analyzing: bool) -> String;
    fn no_indexes_only_id(&self) -> &'static str;
    fn index_statistics_title(&self) -> &'static str;
    fn no_index_selected(&self) -> &'static str;
    fn load_statistics_hint(&self) -> &'static str;
    fn staleness_partial(&self) -> &'static str;
    fn staleness_stale(&self) -> &'static str;
    fn label_keys(&self) -> &'static str;
    fn label_distinct(&self) -> &'static str;
    fn label_histogram(&self) -> &'static str;
    fn create_index_title(&self) -> &'static str;
    fn label_fields_comma(&self) -> &'static str;
    fn label_compound(&self) -> &'static str;
    fn compound_field_count(&self, count: usize) -> String;
    fn label_unique(&self) -> &'static str;
    fn label_sparse(&self) -> &'static str;
    fn sparse_hint(&self) -> &'static str;
    fn stale_indexes(&self, count: usize) -> String;

    // Script modal
    fn confirm_discard_title(&self) -> &'static str;
    fn confirm_discard(&self) -> &'static str;
    fn confirm_delete_script_title(&self) -> &'static str;
    fn confirm_delete_script(&self) -> &'static str;
    fn scripts_title(&self) -> &'static str;
    fn key_edit(&self) -> &'static str;
    fn key_history(&self) -> &'static str;
    fn key_inline(&self) -> &'static str;
    fn key_back(&self) -> &'static str;
    fn key_save_run(&self) -> &'static str;
    fn key_focus(&self) -> &'static str;
    fn key_view(&self) -> &'static str;
    fn no_scripts(&self) -> &'static str;
    fn run_count(&self, count: u64) -> String;
    fn no_description_parens(&self) -> &'static str;
    fn never(&self) -> &'static str;
    fn label_description(&self) -> &'static str;
    fn label_last_run(&self) -> &'static str;
    fn script_editor_new_title(&self) -> &'static str;
    fn script_editor_inline_title(&self) -> &'static str;
    fn script_editor_title(&self) -> &'static str;
    fn label_tags(&self) -> &'static str;
    fn label_code(&self) -> &'static str;
    fn label_params_json(&self) -> &'static str;
    fn script_logs(&self, count: usize, first: &str) -> String;
    fn result_error(&self) -> &'static str;
    fn script_result_placeholder(&self) -> &'static str;
    fn script_history_title(&self, name: &str) -> String;
    fn no_version_history(&self) -> &'static str;
    fn current_marker(&self) -> &'static str;
    fn code_preview_title(&self) -> &'static str;
    /// The script a confirmation is about
    fn script_name_line(&self, name: &str) -> String;

    // RAG modal
    fn rag_title(&self) -> &'static str;
    fn tab_import(&self) -> &'static str;
    fn tab_models(&self) -> &'static str;
    fn tab_jobs(&self) -> &'static str;
    fn label_query(&self) -> &'static str;
    fn label_limit(&self) -> &'static str;
    fn rag_results_title(&self, count: usize, loading: bool, trimmed: bool) -> String;
    fn no_rag_results(&self) -> &'static str;
    fn key_fields(&self) -> &'static str;
    fn key_import(&self) -> &'static str;
    fn label_title_optional(&self) -> &'static str;
    fn title_placeholder(&self) -> &'static str;
    fn content_title(&self, chars: usize) -> String;
    fn content_placeholder(&self) -> &'static str;
    fn label_chunk_size(&self) -> &'static str;
    fn label_overlap(&self) -> &'static str;
    fn key_stats(&self) -> &'static str;
    fn rag_collections_title(&self, count: usize) -> String;
    fn no_rag_collections(&self) -> &'static str;
    fn key_toggle(&self) -> &'static str;
    fn label_collection_name(&self) -> &'static str;
    fn label_embedding_field(&self) -> &'static str;
    fn label_text_field(&self) -> &'static str;
    fn label_provider_space(&self) -> &'static str;
    fn label_language_space(&self) -> &'static str;
    fn key_enable_auto_embed(&self) -> &'static str;
    fn key_cache_stats(&self) -> &'static str;
    fn key_clear_cache(&self) -> &'static str;
    fn key_test_embed(&self) -> &'static str;
    fn embedding_models_title(&self, count: usize) -> String;
    fn loading_models(&self) -> &'static str;
    fn auto_embed_title(&self) -> &'static str;
    fn cache_title(&self) -> &'static str;
    fn key_embed(&self) -> &'static str;
    fn text_to_embed_title(&self) -> &'static str;
    fn type_text_here(&self) -> &'static str;
    fn no_result_yet(&self) -> &'static str;
    fn key_cancel_job(&self) -> &'static str;
    fn jobs_title(&self, count: usize) -> String;
    fn no_jobs(&self) -> &'static str;
    fn import_result(
        &self,
        doc_id: &str,
        created: usize,
        replaced: u64,
        language_ignored: bool,
    ) -> String;
    fn rag_collection_row(
        &self,
        name: &str,
        chunks: &dyn Display,
        documents: &dyn Display,
        provider: Option<&str>,
    ) -> String;
    fn model_row(&self, provider: &str, model: &str, dimension: usize, available: bool) -> String;
    fn not_loaded(&self) -> &'static str;
    fn auto_embed_off(&self) -> &'static str;
    fn auto_embed_status(
        &self,
        enabled: bool,
        source: &str,
        target: &str,
        provider: &str,
        reembed_pending: bool,
        provider_removed: bool,
    ) -> String;
    fn cache_not_loaded(&self) -> &'static str;
    fn cache_disabled_on_server(&self) -> &'static str;
    fn cache_stats(
        &self,
        entries: usize,
        max_entries: usize,
        hit_rate: f64,
        hits: u64,
        misses: u64,
    ) -> String;
    fn embed_result(&self, provider: &str, model: &str, dimension: usize, preview: &str) -> String;
    fn job_pending(&self) -> &'static str;
    fn job_running(&self, progress: f64) -> String;
    fn job_completed(&self) -> &'static str;
    fn job_failed(&self, error: &str) -> String;
    fn job_cancelled(&self) -> &'static str;
    fn job_backfill(&self, collection: &str) -> String;
    fn job_import(&self, collection: &str, doc_id: &str) -> String;
    fn job_compact(&self) -> &'static str;

    // ACL modal
    fn acl_editor_title(&self, collection: &str) -> String;
    fn acl_rules_title(&self) -> &'static str;
    fn key_add(&self) -> &'static str;
    fn no_acl_rules(&self) -> &'static str;
    fn label_collection(&self) -> &'static str;
    fn label_rules(&self) -> &'static str;
    fn label_type(&self) -> &'static str;
    fn acl_builtin(&self) -> &'static str;
    fn acl_custom(&self) -> &'static str;
    fn rule_count(&self, count: usize) -> String;
    fn no_rules_defined(&self) -> &'static str;
    fn label_principal(&self) -> &'static str;
    fn label_permissions(&self) -> &'static str;
    fn rules_title(&self, count: usize) -> String;
    fn no_rules_yet(&self) -> &'static str;
    fn add_rule_title(&self) -> &'static str;
    fn change_hint(&self) -> &'static str;
    fn perm_read(&self) -> &'static str;
    fn perm_write(&self) -> &'static str;
    fn perm_admin(&self) -> &'static str;
    fn confirm_delete_acl(&self, collection: &str) -> String;
    fn confirm_save_acl(&self, collection: &str, rules: usize) -> String;
    fn acl_read_only(&self) -> &'static str;
    fn acl_uneditable(&self, collection: &str, reasons: &str) -> String;
    fn principal_value_empty(&self) -> &'static str;
    fn select_a_permission(&self) -> &'static str;
    fn add_a_rule(&self) -> &'static str;

    // Key handlers
    fn select_collection_first(&self) -> &'static str;
    fn acl_needs_localhost(&self) -> &'static str;
    fn refreshing(&self) -> &'static str;
    fn refreshed(&self) -> &'static str;
    fn filter_cleared(&self) -> &'static str;
    /// Esc stopped the background loads
    fn tasks_cancelled(&self) -> &'static str;
    /// A background load runs (status bar)
    fn loading_in_background(&self) -> &'static str;
    /// The open modal's request was stopped (Esc)
    fn request_stopped(&self) -> &'static str;
    /// An index creation was stopped: the server may still have built it
    fn index_create_stopped(&self) -> &'static str;
    /// An index is being built (index modal; Esc stops it)
    fn creating_index(&self) -> &'static str;
    /// An export runs: documents written so far of the expected count
    fn exporting(&self, written: usize, total: usize) -> String;
    fn acl_deleted(&self, collection: &str) -> String;
    fn acl_not_found(&self) -> &'static str;
    fn acl_saved(&self, collection: &str, rules: usize) -> String;
    fn key_copied(&self) -> &'static str;
    fn api_key_created(&self, name: &str, saved_to: Option<&str>) -> String;
    fn api_key_revoked(&self, name: &str) -> String;
    fn api_key_deleted(&self, name: &str) -> String;
    fn key_not_found(&self) -> &'static str;
    fn path_required(&self) -> &'static str;
    fn database_opened(&self, path: &str) -> String;
    fn database_created(&self, path: &str) -> String;
    fn opened_ok(&self) -> &'static str;
    fn created_ok(&self) -> &'static str;
    fn file_missing_use_create(&self) -> &'static str;
    fn file_exists_use_open(&self) -> &'static str;
    fn no_connection(&self) -> &'static str;
    fn value_required(&self) -> &'static str;
    fn id_required(&self) -> &'static str;
    fn bind_required(&self) -> &'static str;
    fn listener_created(&self, id: &str, replaced: bool, note: &str) -> String;
    fn listener_not_found(&self) -> &'static str;
    fn query_empty(&self) -> &'static str;
    fn content_empty(&self) -> &'static str;
    fn collection_name_required(&self) -> &'static str;
    fn stats_refreshed(&self) -> &'static str;
    fn text_empty(&self) -> &'static str;
    fn auto_embed_enabled(&self) -> &'static str;
    fn job_suffix(&self, job_id: &str) -> String;
    fn warning_suffix(&self, warning: &str) -> String;
    fn cache_stats_loaded(&self) -> &'static str;
    fn jobs_refreshed(&self) -> &'static str;
    fn rag_collection_ready(
        &self,
        collection: &str,
        vector_created: bool,
        fulltext_created: usize,
        dimension: usize,
        provider: &str,
    ) -> String;
    fn dimension_positive(&self) -> &'static str;
    fn version_loaded_readonly(&self, version: u32) -> String;
    fn unknown(&self) -> &'static str;
    // Key map (help, actions menu)
    fn action_label(&self, action: Action) -> &'static str;
    fn section_title(&self, section: Section) -> &'static str;
    fn help_title(&self) -> &'static str;
    fn actions_title(&self) -> &'static str;
    fn help_menu_note(&self) -> &'static str;
    fn vector_index_created(&self, name: &str) -> String;
    fn rag_stats_line(
        &self,
        collection: &str,
        chunks: &dyn Display,
        documents: &dyn Display,
    ) -> String;
}

/// A group of MCP tools in the server info
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolCategory {
    Database,
    Collections,
    Query,
    Insert,
    Update,
    Delete,
    Count,
    Aggregate,
    Indexes,
    Schema,
    Scripts,
    Fuzzy,
    Other,
}

/// `Messages` is implemented only by the languages of this crate: a new text
/// is a required method that every language must provide (a missing
/// translation does not compile), and no other crate can implement the trait,
/// so adding a text never breaks one.
mod sealed {
    pub trait Sealed {}
    impl Sealed for super::Hu {}
    impl Sealed for super::En {}
}

/// An operation a failure message names ([`Messages::failed`])
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op {
    Search,
    Refresh,
    LoadDocuments,
    LoadSchema,
    LoadScripts,
    LoadScript,
    LoadHistory,
    LoadJobs,
    LoadStatistics,
    LoadIndexes,
    Save,
    Create,
    Delete,
    Rollback,
    Insert,
    Update,
    Query,
    Explain,
    Count,
    Filter,
    Analyze,
    Copy,
    File,
    Write,
    Flush,
    LoadApiKeys,
    LoadAcl,
    LoadListeners,
    LoadModels,
    LoadAutoEmbedStatus,
    LoadFulltextIndexes,
    LoadCollections,
    ReloadList,
    Connect,
    CheckUpdate,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_config_value() {
        assert_eq!(Language::parse("HU"), Some(Language::Hu));
        assert_eq!(Language::parse(" en "), Some(Language::En));
        assert_eq!(Language::parse("de"), None);
    }

    #[test]
    fn test_language_of_a_locale() {
        assert_eq!(Language::from_locale("hu_HU.UTF-8"), Language::Hu);
        assert_eq!(Language::from_locale("HU"), Language::Hu);
        assert_eq!(Language::from_locale("en_US.UTF-8"), Language::En);
        assert_eq!(Language::from_locale("C"), Language::En);
    }

    /// Both languages have every text, and they are not left untranslated
    #[test]
    fn test_both_languages_have_their_own_texts() {
        let (hu, en): (&dyn Messages, &dyn Messages) = (&Hu, &En);
        assert_ne!(hu.no_collections(), en.no_collections());
        assert_ne!(
            hu.documents_title(1, 2, true),
            en.documents_title(1, 2, true)
        );
        assert_eq!(en.detail_title(Some((2, 5))), " Details [2/5] ");
        assert_eq!(hu.detail_title(None), " Részletek ");
    }

    /// Every status and error text is translated, not left in one language
    #[test]
    fn test_status_texts_differ_between_the_languages() {
        let (hu, en): (&dyn Messages, &dyn Messages) = (&Hu, &En);
        let fixed: [fn(&dyn Messages) -> &'static str; 26] = [
            |m| m.unexpected_search_response(),
            |m| m.no_database(),
            |m| m.no_collection_selected(),
            |m| m.no_document_selected(),
            |m| m.no_script_selected(),
            |m| m.no_version_selected(),
            |m| m.name_required(),
            |m| m.field_name_required(),
            |m| m.file_path_required(),
            |m| m.no_matches(),
            |m| m.filter_needs_a_condition(),
            |m| m.collection_exists(),
            |m| m.document_deleted(),
            |m| m.document_not_found(),
            |m| m.document_unchanged(),
            |m| m.document_updated(),
            |m| m.document_must_be_object(),
            |m| m.document_id_missing(),
            |m| m.query_plan_loaded(),
            |m| m.nothing_to_export(),
            |m| m.analyzing_indexes(),
            |m| m.index_statistics_refreshed(),
            |m| m.vector_input_empty(),
            |m| m.vector_empty(),
            |m| m.auto_embed_disabled(),
            |m| m.script_name_required(),
        ];
        for text in fixed {
            assert_ne!(text(hu), text(en));
        }
        let ops = [
            Op::Search,
            Op::Refresh,
            Op::LoadDocuments,
            Op::LoadSchema,
            Op::LoadScripts,
            Op::LoadScript,
            Op::LoadHistory,
            Op::LoadJobs,
            Op::LoadStatistics,
            Op::LoadIndexes,
            Op::Save,
            Op::Create,
            Op::Delete,
            Op::Rollback,
            Op::Insert,
            Op::Update,
            Op::Query,
            Op::Explain,
            Op::Count,
            Op::Filter,
            Op::Analyze,
            Op::Copy,
            Op::File,
            Op::Write,
            Op::Flush,
            Op::LoadApiKeys,
            Op::LoadAcl,
            Op::LoadListeners,
            Op::LoadModels,
            Op::LoadAutoEmbedStatus,
            Op::LoadFulltextIndexes,
            Op::LoadCollections,
            Op::ReloadList,
            Op::Connect,
            Op::CheckUpdate,
        ];
        let mut seen = std::collections::HashSet::new();
        for op in ops {
            let (h, e) = (hu.failed(op, &"x"), en.failed(op, &"x"));
            assert!(h.ends_with("sikertelen: x"), "{h}");
            assert!(e.ends_with("failed: x"), "{e}");
            // Each operation has its own name
            assert!(seen.insert(e));
        }
    }

    #[test]
    fn test_status_texts_with_values() {
        let (hu, en): (&dyn Messages, &dyn Messages) = (&Hu, &En);
        assert_eq!(
            hu.failed(Op::Search, &"timeout"),
            "Keresés sikertelen: timeout"
        );
        assert_eq!(en.match_count(1), "1 match");
        assert_eq!(en.match_count(3), "3 matches");
        assert_eq!(hu.match_count(3), "3 találat");
        assert_eq!(en.query_found(5, Some(5)), "5+ documents (limit: 5)");
        assert_eq!(
            en.exported(1, true, "/x.json"),
            "1 document exported (filtered): /x.json"
        );
        assert_eq!(
            hu.index_created("a, b", true, " [unique]", "a_1_b_1"),
            "Összetett index: a, b [unique] létrehozva (a_1_b_1)"
        );
        // The English search summary is the one the RAG modal showed before
        assert_eq!(en.search_summary(2, 0, 0, false), "Found 2 results");
        assert_eq!(
            en.search_summary(1, 2, 1, true),
            "Found 1 result (trimmed: 2 more over the context budget, 1 without body text)"
        );
        assert_eq!(
            en.search_summary(1, 0, 0, true),
            "Found 1 result (trimmed: passages shortened)"
        );
        assert_eq!(
            hu.search_summary(1, 0, 0, true),
            "1 találat (kimaradt: rövidített szakaszok)"
        );
    }

    /// The modals' texts are translated, not left in one language
    #[test]
    fn test_modal_texts_differ_between_the_languages() {
        let (hu, en): (&dyn Messages, &dyn Messages) = (&Hu, &En);
        let fixed: [fn(&dyn Messages) -> &'static str; 68] = [
            |m| m.key_save(),
            |m| m.key_cancel(),
            |m| m.key_close(),
            |m| m.key_create(),
            |m| m.key_search(),
            |m| m.key_search_or_jump(),
            |m| m.key_format(),
            |m| m.key_export(),
            |m| m.key_switch_mode(),
            |m| m.key_switch_field(),
            |m| m.key_navigate(),
            |m| m.key_indent(),
            |m| m.label_error(),
            |m| m.label_name(),
            |m| m.label_file(),
            |m| m.label_format(),
            |m| m.label_path(),
            |m| m.label_mode(),
            |m| m.label_field(),
            |m| m.error_details_title(),
            |m| m.esc_to_close(),
            |m| m.new_collection_title(),
            |m| m.collection_name_hint(),
            |m| m.edit_document_title(),
            |m| m.new_document_title(),
            |m| m.update_check_title(),
            |m| m.checking_for_updates(),
            |m| m.current_version(),
            |m| m.latest_version(),
            |m| m.update_available(),
            |m| m.update_instructions(),
            |m| m.whats_new(),
            |m| m.up_to_date(),
            |m| m.export_title(),
            |m| m.press_enter_to_export(),
            |m| m.search_collections_title(),
            |m| m.search_document_title(),
            |m| m.type_collection_name(),
            |m| m.type_search_text(),
            |m| m.start_typing_to_search(),
            |m| m.open_database_title(),
            |m| m.create_database_title(),
            |m| m.mode_open(),
            |m| m.mode_create(),
            |m| m.open_database_info(),
            |m| m.create_database_info(),
            |m| m.connecting(),
            |m| m.open_database_tip(),
            |m| m.create_database_tip(),
            |m| m.fulltext_title(),
            |m| m.no_fulltext_index(),
            |m| m.searching(),
            |m| m.confirm_title(),
            |m| m.confirm_message(),
            |m| m.yes(),
            |m| m.confirm_delete_document_title(),
            |m| m.confirm_drop_collection_title(),
            |m| m.confirm_drop_index_title(),
            |m| m.confirm_drop_vector_index_title(),
            |m| m.confirm_rollback_title(),
            |m| m.rollback_button(),
            |m| m.confirm_disable_auto_embed_title(),
            |m| m.disable_button(),
            |m| m.confirm_clear_cache_title(),
            |m| m.confirm_clear_cache(),
            |m| m.clear_button(),
            |m| m.confirm_cancel_job_title(),
            |m| m.cancel_job_button(),
        ];
        for text in fixed {
            assert_ne!(text(hu), text(en));
        }
        assert_ne!(
            hu.confirm_drop_collection("a", 2),
            en.confirm_drop_collection("a", 2)
        );
        assert_eq!(en.collection_with_count("a", 1), "a (1 document)");
        assert!(en
            .confirm_drop_collection("a", 1)
            .ends_with("1 document will be deleted!"));
        assert!(en
            .confirm_drop_collection("a", 2)
            .ends_with("2 documents will be deleted!"));
        assert_eq!(hu.results_title(50, true), " Találatok (50+) ");
        assert_eq!(
            en.export_collection_info("a", 2, true),
            "Collection: a (2 documents, filtered)"
        );
    }

    /// The filter, query and server info texts are translated, and so are
    /// the operator, sort, template and tool category names that have words
    #[test]
    fn test_filter_query_server_info_texts_differ_between_the_languages() {
        use crate::state::filter::{FilterOperator, SortDirection};
        use crate::state::query::QueryTemplate;
        let (hu, en): (&dyn Messages, &dyn Messages) = (&Hu, &En);
        let fixed: [fn(&dyn Messages) -> &'static str; 50] = [
            |m| m.filter_title(),
            |m| m.key_choose_field(),
            |m| m.key_next(),
            |m| m.field_placeholder_examples(),
            |m| m.field_placeholder_pick(),
            |m| m.label_operator(),
            |m| m.label_value(),
            |m| m.value_placeholder(),
            |m| m.hint_type_field(),
            |m| m.hint_field_next(),
            |m| m.hint_operator(),
            |m| m.hint_add_exists(),
            |m| m.hint_type_value(),
            |m| m.hint_add_condition(),
            |m| m.hint_filters(),
            |m| m.hint_sort(),
            |m| m.no_active_filters(),
            |m| m.label_sort(),
            |m| m.no_sort(),
            |m| m.press_ctrl_s_to_search(),
            |m| m.add_filters(),
            |m| m.query_title(),
            |m| m.key_run(),
            |m| m.key_plan(),
            |m| m.key_templates(),
            |m| m.label_result(),
            |m| m.query_prompt(),
            |m| m.query_plan_title(),
            |m| m.label_plan(),
            |m| m.label_estimated_rows(),
            |m| m.label_stage(),
            |m| m.label_cost(),
            |m| m.label_reason(),
            |m| m.no_query_plan(),
            |m| m.query_plan_toggle_hint(),
            |m| m.server_info_title(),
            |m| m.loading(),
            |m| m.database_info(),
            |m| m.label_server(),
            |m| m.label_api_key(),
            |m| m.api_key_required(),
            |m| m.api_key_not_required(),
            |m| m.label_database(),
            |m| m.label_file_size(),
            |m| m.label_collection_count(),
            |m| m.label_total_documents(),
            |m| m.label_total_indexes(),
            |m| m.label_of_which_vector(),
            |m| m.label_collections(),
            |m| m.no_description(),
        ];
        for text in fixed {
            assert_ne!(text(hu), text(en));
        }
        for operator in [
            FilterOperator::Contains,
            FilterOperator::StartsWith,
            FilterOperator::Exists,
            FilterOperator::In,
        ] {
            assert_ne!(hu.filter_operator(operator), en.filter_operator(operator));
        }
        // A symbol reads the same in both
        assert_eq!(hu.filter_operator(FilterOperator::GreaterOrEq), ">=");
        assert_eq!(en.filter_operator(FilterOperator::GreaterOrEq), ">=");
        for direction in [SortDirection::Asc, SortDirection::Desc] {
            assert_ne!(hu.sort_direction(direction), en.sort_direction(direction));
        }
        for template in [
            QueryTemplate::All,
            QueryTemplate::ById,
            QueryTemplate::Exists,
        ] {
            assert_ne!(hu.query_template(template), en.query_template(template));
        }
        for category in [ToolCategory::Database, ToolCategory::Other] {
            assert_ne!(hu.tool_category(category), en.tool_category(category));
        }
        assert_eq!(en.field_title(1), " Field (1 field) [↑↓] ");
        assert_eq!(hu.field_title(0), " Mező ");
        assert_eq!(en.vector_index_summary(1, 3), "1 (3 vectors)");
        // The exact texts, not only that they differ
        assert_eq!(hu.vector_index_summary(1, 3), "1 (3 vektor)");
        assert_eq!(hu.filter_operator(FilterOperator::Contains), "tartalmazza");
        assert_eq!(
            en.filter_operator(FilterOperator::StartsWith),
            "starts with"
        );
        assert_eq!(hu.sort_direction(SortDirection::Desc), "↓ csökkenő");
        assert_eq!(en.query_template(QueryTemplate::ById), "By _id");
    }

    /// The API key, listener, vector and index texts are translated (they
    /// were English only, or Hungarian without accents)
    #[test]
    fn test_admin_vector_index_texts_differ_between_the_languages() {
        let (hu, en): (&dyn Messages, &dyn Messages) = (&Hu, &En);
        let fixed: [fn(&dyn Messages) -> &'static str; 62] = [
            |m| m.no(),
            |m| m.key_new(),
            |m| m.key_tabs(),
            |m| m.key_next_field(),
            |m| m.label_status(),
            |m| m.label_field_name(),
            |m| m.status_active(),
            |m| m.api_keys_title(),
            |m| m.key_revoke(),
            |m| m.admin_needs_localhost(),
            |m| m.admin_needs_key(),
            |m| m.new_key_label(),
            |m| m.new_key_warning(),
            |m| m.no_api_keys(),
            |m| m.label_key(),
            |m| m.status_revoked(),
            |m| m.enter_new_key_name(),
            |m| m.listeners_title(),
            |m| m.key_enable(),
            |m| m.key_disable(),
            |m| m.restart_needed(),
            |m| m.no_listeners(),
            |m| m.label_bind(),
            |m| m.status_disabled(),
            |m| m.enter_listener_details(),
            |m| m.label_bind_address(),
            |m| m.checkbox_enabled(),
            |m| m.checkbox_disabled(),
            |m| m.space_to_toggle(),
            |m| m.listener_read_only(),
            |m| m.vector_search_title(),
            |m| m.tab_create_index(),
            |m| m.tab_list_indexes(),
            |m| m.label_k_limit(),
            |m| m.label_vector_json(),
            |m| m.label_filter_optional(),
            |m| m.no_vector_results(),
            |m| m.key_toggle_metric(),
            |m| m.label_dimension(),
            |m| m.label_distance_metric(),
            |m| m.metric_cosine(),
            |m| m.metric_euclidean(),
            |m| m.metric_dot_product(),
            |m| m.dimension_tip(),
            |m| m.no_vector_indexes(),
            |m| m.index_title(),
            |m| m.key_analyze(),
            |m| m.no_indexes_only_id(),
            |m| m.index_statistics_title(),
            |m| m.no_index_selected(),
            |m| m.load_statistics_hint(),
            |m| m.staleness_partial(),
            |m| m.staleness_stale(),
            |m| m.label_keys(),
            |m| m.label_distinct(),
            |m| m.label_histogram(),
            |m| m.create_index_title(),
            |m| m.label_fields_comma(),
            |m| m.label_compound(),
            |m| m.label_unique(),
            |m| m.label_sparse(),
            |m| m.sparse_hint(),
        ];
        for text in fixed {
            assert_ne!(text(hu), text(en));
        }
        assert_eq!(
            hu.confirm_revoke_key("ci"),
            "Biztosan visszavonod a(z) „ci” API-kulcsot?"
        );
        assert_eq!(
            en.confirm_delete_listener("lan"),
            "DELETE the listener \"lan\"?"
        );
        assert_eq!(
            hu.vector_index_row("v", "emb", 3, "cosine", 2),
            "v | emb | dim:3 | cosine | 2 vektor"
        );
        assert_eq!(
            en.vector_index_row("v", "emb", 3, "cosine", 1),
            "v | emb | dim:3 | cosine | 1 vector"
        );
        assert_eq!(en.stale_indexes(1), "1 stale index – [a] Analyze");
        assert_eq!(hu.stale_indexes(2), "2 elavult index – [a] Elemzés");
        assert_eq!(en.compound_field_count(3), " [3 fields]");
        assert_eq!(
            hu.existing_indexes_title(true),
            " Meglévő indexek (elemzés...) "
        );
    }

    /// The script and RAG texts are translated; the English ones read as the
    /// RAG modal's did before (it was English only)
    #[test]
    fn test_script_rag_texts_differ_between_the_languages() {
        let (hu, en): (&dyn Messages, &dyn Messages) = (&Hu, &En);
        let fixed: [fn(&dyn Messages) -> &'static str; 71] = [
            |m| m.confirm_discard_title(),
            |m| m.confirm_discard(),
            |m| m.confirm_delete_script_title(),
            |m| m.confirm_delete_script(),
            |m| m.scripts_title(),
            |m| m.key_edit(),
            |m| m.key_history(),
            |m| m.key_inline(),
            |m| m.key_back(),
            |m| m.key_save_run(),
            |m| m.key_focus(),
            |m| m.key_view(),
            |m| m.no_scripts(),
            |m| m.no_description_parens(),
            |m| m.never(),
            |m| m.label_description(),
            |m| m.label_last_run(),
            |m| m.script_editor_new_title(),
            |m| m.script_editor_inline_title(),
            |m| m.script_editor_title(),
            |m| m.label_tags(),
            |m| m.label_code(),
            |m| m.label_params_json(),
            |m| m.result_error(),
            |m| m.script_result_placeholder(),
            |m| m.no_version_history(),
            |m| m.current_marker(),
            |m| m.code_preview_title(),
            |m| m.rag_title(),
            |m| m.tab_import(),
            |m| m.tab_models(),
            |m| m.tab_jobs(),
            |m| m.label_query(),
            |m| m.label_limit(),
            |m| m.no_rag_results(),
            |m| m.key_fields(),
            |m| m.key_import(),
            |m| m.label_title_optional(),
            |m| m.title_placeholder(),
            |m| m.content_placeholder(),
            |m| m.label_chunk_size(),
            |m| m.label_overlap(),
            |m| m.key_stats(),
            |m| m.no_rag_collections(),
            |m| m.key_toggle(),
            |m| m.label_collection_name(),
            |m| m.label_embedding_field(),
            |m| m.label_text_field(),
            |m| m.label_provider_space(),
            |m| m.label_language_space(),
            |m| m.key_enable_auto_embed(),
            |m| m.key_cache_stats(),
            |m| m.key_clear_cache(),
            |m| m.key_test_embed(),
            |m| m.loading_models(),
            |m| m.auto_embed_title(),
            |m| m.cache_title(),
            |m| m.key_embed(),
            |m| m.text_to_embed_title(),
            |m| m.type_text_here(),
            |m| m.no_result_yet(),
            |m| m.key_cancel_job(),
            |m| m.no_jobs(),
            |m| m.not_loaded(),
            |m| m.auto_embed_off(),
            |m| m.cache_not_loaded(),
            |m| m.cache_disabled_on_server(),
            |m| m.job_pending(),
            |m| m.job_completed(),
            |m| m.job_cancelled(),
            |m| m.job_compact(),
        ];
        for text in fixed {
            assert_ne!(text(hu), text(en));
        }
        assert_eq!(
            en.import_result("d1", 3, 1, false),
            "Imported \"d1\": 3 chunks created, 1 replaced"
        );
        assert_eq!(
            en.import_result("d1", 1, 0, false),
            "Imported \"d1\": 1 chunk created"
        );
        assert_eq!(
            en.rag_collection_row("users", &6, &4, Some("length")),
            "users | 6 chunks | 4 docs | length"
        );
        assert_eq!(
            hu.rag_collection_row("users", &6, &4, None),
            "users | 6 darab | 4 dokumentum | nincs RAG-konfiguráció"
        );
        assert_eq!(
            en.model_row("vllm", "m", 2, false),
            "vllm | m | dim:2 | N/A"
        );
        assert_eq!(
            en.cache_stats(1, 100, 50.0, 1, 1),
            "1/100 entries | 50.0% hit rate (1 hits, 1 misses)"
        );
        assert_eq!(
            en.auto_embed_status(true, "content", "embedding", "vllm", true, true),
            "Enabled: content -> embedding (vllm) | re-embed pending | provider removed: enable again"
        );
        assert_eq!(en.job_running(25.0), "running 25%");
        assert_eq!(en.job_backfill("users"), "backfill users");
        assert_eq!(
            en.rag_results_title(1, false, true),
            " Results (1, trimmed) "
        );
        assert_eq!(en.run_count(1), "    1 run ");
        assert_eq!(hu.run_count(12), "   12 futás");
    }

    /// The ACL modal's texts are translated (it was English only); the
    /// principal types and permission names stay, they are the server's
    #[test]
    fn test_acl_texts_differ_between_the_languages() {
        let (hu, en): (&dyn Messages, &dyn Messages) = (&Hu, &En);
        let fixed: [fn(&dyn Messages) -> &'static str; 21] = [
            |m| m.acl_rules_title(),
            |m| m.key_add(),
            |m| m.no_acl_rules(),
            |m| m.label_collection(),
            |m| m.label_rules(),
            |m| m.label_type(),
            |m| m.acl_builtin(),
            |m| m.acl_custom(),
            |m| m.no_rules_defined(),
            |m| m.label_principal(),
            |m| m.label_permissions(),
            |m| m.no_rules_yet(),
            |m| m.add_rule_title(),
            |m| m.change_hint(),
            |m| m.perm_read(),
            |m| m.perm_write(),
            |m| m.perm_admin(),
            |m| m.acl_read_only(),
            |m| m.principal_value_empty(),
            |m| m.select_a_permission(),
            |m| m.add_a_rule(),
        ];
        for text in fixed {
            assert_ne!(text(hu), text(en));
        }
        assert_eq!(
            en.acl_uneditable("users", "principal anyone"),
            "The ACL for \"users\" contains rules this editor cannot edit yet: principal anyone"
        );
        assert_eq!(
            en.confirm_save_acl("users", 1),
            "Save the ACL for \"users\"?\n\n1 rule will be applied."
        );
        assert_eq!(en.rule_count(2), "2 rules");
        assert_eq!(hu.rule_count(1), "1 szabály");
        assert_eq!(hu.acl_editor_title("users"), "ACL-szerkesztő: users");
    }

    /// The key handlers' messages are translated (they mixed English and
    /// Hungarian without accents)
    #[test]
    fn test_handler_texts_differ_between_the_languages() {
        let (hu, en): (&dyn Messages, &dyn Messages) = (&Hu, &En);
        let fixed: [fn(&dyn Messages) -> &'static str; 28] = [
            |m| m.select_collection_first(),
            |m| m.acl_needs_localhost(),
            |m| m.refreshing(),
            |m| m.refreshed(),
            |m| m.filter_cleared(),
            |m| m.acl_not_found(),
            |m| m.key_copied(),
            |m| m.key_not_found(),
            |m| m.path_required(),
            |m| m.opened_ok(),
            |m| m.created_ok(),
            |m| m.file_missing_use_create(),
            |m| m.file_exists_use_open(),
            |m| m.no_connection(),
            |m| m.value_required(),
            |m| m.id_required(),
            |m| m.bind_required(),
            |m| m.listener_not_found(),
            |m| m.query_empty(),
            |m| m.content_empty(),
            |m| m.collection_name_required(),
            |m| m.stats_refreshed(),
            |m| m.text_empty(),
            |m| m.auto_embed_enabled(),
            |m| m.cache_stats_loaded(),
            |m| m.jobs_refreshed(),
            |m| m.dimension_positive(),
            |m| m.unknown(),
        ];
        for text in fixed {
            assert_ne!(text(hu), text(en));
        }
        assert_eq!(en.acl_saved("users", 1), "ACL for \"users\" saved (1 rule)");
        assert_eq!(
            hu.api_key_created("ci", Some("/k.txt")),
            "A(z) „ci” API-kulcs létrehozva, mentve ide: /k.txt"
        );
        assert_eq!(
            en.listener_created("web", true, "Restart."),
            "Listener \"web\" replaced. Restart."
        );
        assert_eq!(
            en.rag_collection_ready("kb", false, 1, 768, "ollama"),
            "RAG collection \"kb\" ready: vector index already existed, 1 fulltext index created (768 dims, ollama)"
        );
        assert_eq!(
            hu.failed(Op::LoadAcl, &"x"),
            "Az ACL betöltése sikertelen: x"
        );
    }

    /// Every action and section of the key map has a text in both languages
    #[test]
    fn test_key_map_texts_differ_between_the_languages() {
        use crate::keymap::{ACTIONS_MENU, MAIN};
        let (hu, en): (&dyn Messages, &dyn Messages) = (&Hu, &En);
        let actions = MAIN
            .iter()
            .map(|binding| binding.action)
            .chain(ACTIONS_MENU.iter().map(|(_, action)| *action));
        for action in actions {
            assert_ne!(
                hu.action_label(action),
                en.action_label(action),
                "{action:?}"
            );
        }
        for section in [Section::Navigation, Section::General, Section::Documents] {
            assert_ne!(hu.section_title(section), en.section_title(section));
        }
        assert_ne!(hu.help_title(), en.help_title());
        assert_ne!(hu.actions_title(), en.actions_title());
    }
}
