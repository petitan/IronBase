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
}
