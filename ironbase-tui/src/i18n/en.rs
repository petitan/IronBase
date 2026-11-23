//! English texts

use super::{Messages, Op};
use std::fmt::Display;

pub struct En;

impl Messages for En {
    fn header_db(&self, version: &str, db: &str, collections: usize, docs: usize) -> String {
        format!(" IronBase v{version} | {db} ({collections} coll, {docs} docs) ")
    }
    fn pane_collections(&self) -> &'static str {
        "Collections"
    }
    fn pane_documents(&self) -> &'static str {
        "Documents"
    }
    fn pane_detail(&self) -> &'static str {
        "Details"
    }

    fn cmd_panel(&self) -> &'static str {
        "Panel"
    }
    fn cmd_navigate(&self) -> &'static str {
        "Navigate"
    }
    fn cmd_select(&self) -> &'static str {
        "Select"
    }
    fn cmd_page(&self) -> &'static str {
        "Page"
    }
    fn cmd_scroll(&self) -> &'static str {
        "Scroll"
    }
    fn cmd_refresh(&self) -> &'static str {
        "Refresh"
    }
    fn cmd_filter(&self) -> &'static str {
        "Filter"
    }
    fn cmd_search(&self) -> &'static str {
        "Search"
    }
    fn cmd_actions(&self) -> &'static str {
        "Actions"
    }
    fn cmd_edit(&self) -> &'static str {
        "Edit"
    }
    fn cmd_delete(&self) -> &'static str {
        "Delete"
    }
    fn cmd_copy(&self) -> &'static str {
        "Copy"
    }
    fn cmd_help(&self) -> &'static str {
        "Help"
    }
    fn cmd_quit(&self) -> &'static str {
        "Quit"
    }
    fn bar_error(&self, error: &str) -> String {
        format!(" Error: {error}")
    }
    fn bar_error_details(&self) -> &'static str {
        " Details"
    }

    fn collections_title(&self) -> &'static str {
        " Collections "
    }
    fn no_collections(&self) -> &'static str {
        "No collections"
    }
    fn indexes_title(&self) -> &'static str {
        " Indexes "
    }
    fn no_indexes(&self) -> &'static str {
        "No indexes"
    }
    fn documents_title(&self, page: usize, pages: usize, filtered: bool) -> String {
        let filter = if filtered {
            " [FILTERED – Esc: clear filter]"
        } else {
            ""
        };
        format!(" Documents (page {page}/{pages}){filter} ")
    }
    fn no_documents(&self) -> &'static str {
        "No documents"
    }
    fn select_collection(&self) -> &'static str {
        "Select a collection"
    }
    fn detail_title(&self, matches: Option<(usize, usize)>) -> String {
        match matches {
            Some((current, total)) => format!(" Details [{current}/{total}] "),
            None => " Details ".to_string(),
        }
    }
    fn select_document(&self) -> &'static str {
        "Select a document"
    }

    fn no_database(&self) -> &'static str {
        "No database is open"
    }
    fn no_collection_selected(&self) -> &'static str {
        "No collection selected"
    }
    fn no_document_selected(&self) -> &'static str {
        "No document selected"
    }
    fn no_script_selected(&self) -> &'static str {
        "No script selected"
    }
    fn no_version_selected(&self) -> &'static str {
        "No version selected"
    }
    fn failed(&self, op: Op, error: &dyn Display) -> String {
        let op = match op {
            Op::Search => "Search",
            Op::Refresh => "Refresh",
            Op::LoadDocuments => "Loading the documents",
            Op::LoadSchema => "Loading the schema",
            Op::LoadScripts => "Loading the script list",
            Op::LoadScript => "Loading the script",
            Op::LoadHistory => "Loading the history",
            Op::LoadJobs => "Loading the jobs",
            Op::LoadStatistics => "Loading the statistics",
            Op::LoadIndexes => "Loading the indexes",
            Op::Save => "Saving",
            Op::Delete => "Deleting",
            Op::Create => "Creating",
            Op::Rollback => "Rollback",
            Op::Insert => "Insert",
            Op::Update => "Update",
            Op::Query => "Query",
            Op::Explain => "Query plan",
            Op::Count => "Count",
            Op::Filter => "Filter",
            Op::Analyze => "Index analysis",
            Op::Copy => "Copying to the clipboard",
            Op::File => "File operation",
            Op::Write => "Writing",
            Op::Flush => "Flushing to disk",
        };
        format!("{op} failed: {error}")
    }
    fn error(&self, error: &dyn Display) -> String {
        format!("Error: {error}")
    }
    fn clipboard_unavailable(&self, error: &dyn Display) -> String {
        format!("The clipboard is not available: {error}")
    }
    fn invalid_json(&self, error: &dyn Display) -> String {
        format!("Invalid JSON: {error}")
    }
    fn invalid_json_params(&self, error: &dyn Display) -> String {
        format!("Invalid JSON parameters: {error}")
    }
    fn name_required(&self) -> &'static str {
        "A name is required"
    }
    fn field_name_required(&self) -> &'static str {
        "A field name is required"
    }
    fn file_path_required(&self) -> &'static str {
        "A file path is required"
    }
    fn invalid_path(&self, path: &str) -> String {
        format!("Invalid path: {path}")
    }
    fn theme_changed(&self, theme: &str) -> String {
        format!("Theme: {theme}")
    }
    fn copied(&self, preview: &str) -> String {
        format!("Copied to the clipboard: {preview}...")
    }

    fn no_matches(&self) -> &'static str {
        "No matches"
    }
    fn match_count(&self, count: usize) -> String {
        match count {
            1 => "1 match".to_string(),
            _ => format!("{count} matches"),
        }
    }
    fn match_count_with_hint(&self, count: usize) -> String {
        format!("{} (n/N: next/previous)", self.match_count(count))
    }
    fn match_position(&self, current: usize, total: usize) -> String {
        format!("Match {current}/{total}")
    }
    fn filter_needs_a_condition(&self) -> &'static str {
        "Add at least one condition"
    }
    fn filter_no_matches(&self, query: &str) -> String {
        format!("0 matches. Query: {query}")
    }

    fn collection_exists(&self) -> &'static str {
        "This collection already exists"
    }
    fn collection_created(&self, name: &str) -> String {
        format!("Collection created: {name}")
    }
    fn collection_dropped(&self, name: &str) -> String {
        format!("Collection dropped: {name}")
    }
    fn document_deleted(&self) -> &'static str {
        "Document deleted"
    }
    fn document_not_found(&self) -> &'static str {
        "Document not found"
    }
    fn document_unchanged(&self) -> &'static str {
        "Document unchanged (nothing to modify)"
    }
    fn document_updated(&self) -> &'static str {
        "Document updated"
    }
    fn document_updated_vector_stale(&self, error: &dyn Display) -> String {
        format!("Document updated, but re-embedding it failed (the vector is stale): {error}")
    }
    fn document_inserted(&self, id: &dyn Display) -> String {
        format!("Document inserted, _id: {id}")
    }
    fn document_must_be_object(&self) -> &'static str {
        "The document must be an object"
    }
    fn document_id_missing(&self) -> &'static str {
        "The original document's _id is missing"
    }
    fn query_found(&self, count: usize, limit: Option<usize>) -> String {
        match limit {
            Some(limit) => format!("{count}+ documents (limit: {limit})"),
            None if count == 1 => "1 document found".to_string(),
            None => format!("{count} documents found"),
        }
    }
    fn query_plan_loaded(&self) -> &'static str {
        "Query plan loaded"
    }
    fn exported(&self, count: usize, filtered: bool, path: &str) -> String {
        let filtered = if filtered { " (filtered)" } else { "" };
        let documents = if count == 1 { "document" } else { "documents" };
        format!("{count} {documents} exported{filtered}: {path}")
    }
    fn nothing_to_export(&self) -> &'static str {
        "No documents to export"
    }

    fn primary_index_not_droppable(&self, index: &str) -> String {
        format!("The primary _id index cannot be dropped: {index}")
    }
    fn index_created(&self, fields: &str, compound: bool, flags: &str, name: &str) -> String {
        let kind = if compound { "Compound index" } else { "Index" };
        format!("{kind}: {fields}{flags} created ({name})")
    }
    fn index_dropped(&self, name: &str) -> String {
        format!("Index dropped: {name}")
    }
    fn analyzing_indexes(&self) -> &'static str {
        "Analyzing indexes..."
    }
    fn index_statistics_refreshed(&self) -> &'static str {
        "Index statistics refreshed"
    }
    fn vector_input_empty(&self) -> &'static str {
        "No vector given"
    }
    fn vector_empty(&self) -> &'static str {
        "The vector is empty"
    }
    fn invalid_vector(&self, error: &dyn Display) -> String {
        format!("Invalid vector JSON: {error}")
    }
    fn invalid_filter(&self, error: &dyn Display) -> String {
        format!("Invalid filter JSON: {error}")
    }

    fn auto_embed_disabled(&self) -> &'static str {
        "Auto-embed disabled"
    }
    fn search_summary(
        &self,
        found: usize,
        over_budget: usize,
        without_body: usize,
        trimmed: bool,
    ) -> String {
        let found = match found {
            1 => "Found 1 result".to_string(),
            _ => format!("Found {found} results"),
        };
        let mut left_out = Vec::new();
        if over_budget > 0 {
            left_out.push(format!("{over_budget} more over the context budget"));
        }
        if without_body > 0 {
            left_out.push(format!("{without_body} without body text"));
        }
        if left_out.is_empty() && trimmed {
            left_out.push("passages shortened".to_string());
        }
        if left_out.is_empty() {
            found
        } else {
            format!("{found} (trimmed: {})", left_out.join(", "))
        }
    }

    fn script_name_required(&self) -> &'static str {
        "A script name is required"
    }
    fn script_saved(&self, version: u32) -> String {
        format!("Script saved (v{version})")
    }
    fn script_deleted(&self, name: &str) -> String {
        format!("Script deleted: {name}")
    }
    fn script_rolled_back(&self, version: u32) -> String {
        format!("Rolled back, new version: v{version}")
    }
    fn unexpected_search_response(&self) -> &'static str {
        "The search did not answer in the requested form (a document list)"
    }

    fn key_save(&self) -> &'static str {
        "Save"
    }
    fn key_cancel(&self) -> &'static str {
        "Cancel"
    }
    fn key_close(&self) -> &'static str {
        "Close"
    }
    fn key_create(&self) -> &'static str {
        "Create"
    }
    fn key_search(&self) -> &'static str {
        "Search"
    }
    fn key_search_or_jump(&self) -> &'static str {
        "Search/jump"
    }
    fn key_format(&self) -> &'static str {
        "Format"
    }
    fn key_export(&self) -> &'static str {
        "Export"
    }
    fn key_switch_mode(&self) -> &'static str {
        "Switch mode"
    }
    fn key_switch_field(&self) -> &'static str {
        "Switch field"
    }
    fn key_navigate(&self) -> &'static str {
        "Navigate"
    }
    fn key_indent(&self) -> &'static str {
        "2 spaces"
    }
    fn label_error(&self) -> &'static str {
        "Error"
    }
    fn label_name(&self) -> &'static str {
        "Name"
    }
    fn label_file(&self) -> &'static str {
        "File"
    }
    fn label_format(&self) -> &'static str {
        "Format"
    }
    fn label_path(&self) -> &'static str {
        "Path"
    }
    fn label_mode(&self) -> &'static str {
        "Mode"
    }
    fn label_field(&self) -> &'static str {
        "Field"
    }
    fn collection_label(&self, name: &str) -> String {
        format!("Collection: {name}")
    }
    fn error_details_title(&self) -> &'static str {
        "Error details"
    }
    fn error_scroll_hint(&self, current: usize, total: usize) -> String {
        format!("({current}/{total}) – ↑/↓: scroll, Esc: close")
    }
    fn esc_to_close(&self) -> &'static str {
        "Esc: close"
    }
    fn new_collection_title(&self) -> &'static str {
        "New collection"
    }
    fn collection_name_hint(&self) -> &'static str {
        "Enter a name (a-z, 0-9, _, -)"
    }
    fn edit_document_title(&self) -> &'static str {
        "Edit document"
    }
    fn new_document_title(&self) -> &'static str {
        "New document"
    }
    fn cursor_position(&self, line: usize, column: usize) -> String {
        format!("Line: {line} Column: {column}")
    }
    fn update_check_title(&self) -> &'static str {
        "Update check [Esc: close]"
    }
    fn checking_for_updates(&self) -> &'static str {
        "Checking for updates..."
    }
    fn current_version(&self) -> &'static str {
        "Current version"
    }
    fn latest_version(&self) -> &'static str {
        "Latest version"
    }
    fn update_available(&self) -> &'static str {
        "A new version is available!"
    }
    fn update_instructions(&self) -> &'static str {
        "To update, run in PowerShell:"
    }
    fn whats_new(&self) -> &'static str {
        "What's new:"
    }
    fn up_to_date(&self) -> &'static str {
        "You are running the latest version!"
    }
    fn export_title(&self) -> &'static str {
        "Export"
    }
    fn export_collection_info(&self, collection: &str, count: usize, filtered: bool) -> String {
        {
            let filtered = if filtered { ", filtered" } else { "" };
            let documents = if count == 1 { "document" } else { "documents" };
            format!("Collection: {collection} ({count} {documents}{filtered})")
        }
    }
    fn press_enter_to_export(&self) -> &'static str {
        "Press Enter to export"
    }
    fn search_collections_title(&self) -> &'static str {
        "Search collections"
    }
    fn search_document_title(&self) -> &'static str {
        "Search in the document"
    }
    fn type_collection_name(&self) -> &'static str {
        "Type a collection name..."
    }
    fn type_search_text(&self) -> &'static str {
        "Type the text to search for..."
    }
    fn collection_with_count(&self, name: &str, documents: usize) -> String {
        {
            let label = if documents == 1 {
                "document"
            } else {
                "documents"
            };
            format!("{name} ({documents} {label})")
        }
    }
    fn start_typing_to_search(&self) -> &'static str {
        "Start typing to search"
    }
    fn document_match_help(&self, current: usize, total: usize) -> String {
        format!("Match: {current}/{total}\n\n[n] Next  [N] Previous\n[Esc] Close (the matches stay highlighted)")
    }
    fn open_database_title(&self) -> &'static str {
        "Open a database"
    }
    fn create_database_title(&self) -> &'static str {
        "Create a new database"
    }
    fn mode_open(&self) -> &'static str {
        "Open"
    }
    fn mode_create(&self) -> &'static str {
        "Create"
    }
    fn open_database_info(&self) -> &'static str {
        "Enter the path of an existing database file."
    }
    fn create_database_info(&self) -> &'static str {
        "Enter the path of the new database file.\nThe file must not exist yet!"
    }
    fn connecting(&self) -> &'static str {
        "Connecting..."
    }
    fn open_database_tip(&self) -> &'static str {
        "Tip: use an absolute path (e.g. /home/user/data.mlite)"
    }
    fn create_database_tip(&self) -> &'static str {
        "Tip: the .mlite extension is recommended"
    }
    fn fulltext_title(&self) -> &'static str {
        "Full-text search"
    }
    fn no_fulltext_index(&self) -> &'static str {
        "No full-text index – create one first."
    }
    fn results_title(&self, count: usize, more: bool) -> String {
        {
            let more = if more { "+" } else { "" };
            format!(" Results ({count}{more}) ")
        }
    }
    fn searching(&self) -> &'static str {
        "Searching..."
    }
    fn confirm_title(&self) -> &'static str {
        "Confirmation"
    }
    fn confirm_message(&self) -> &'static str {
        "Are you sure you want to continue?"
    }
    fn yes(&self) -> &'static str {
        "Yes"
    }
    fn confirm_delete_document_title(&self) -> &'static str {
        "Delete document"
    }
    fn confirm_delete_document(&self, collection: &str, preview: &str) -> String {
        format!("Delete the document from the \"{collection}\" collection?\n\n{preview}")
    }
    fn confirm_drop_collection_title(&self) -> &'static str {
        "Drop collection"
    }
    fn confirm_drop_collection(&self, name: &str, documents: usize) -> String {
        {
            let will_be = if documents == 1 {
                "document will be"
            } else {
                "documents will be"
            };
            format!("Drop the \"{name}\" collection?\n\n{documents} {will_be} deleted!")
        }
    }
    fn confirm_drop_index_title(&self) -> &'static str {
        "Drop index"
    }
    fn confirm_drop_index(&self, index: &str, collection: &str) -> String {
        format!("Drop the \"{index}\" index?\n\nCollection: {collection}")
    }
    fn confirm_drop_vector_index_title(&self) -> &'static str {
        "Drop vector index"
    }
    fn confirm_drop_vector_index(&self, index: &str, collection: &str) -> String {
        format!("Drop the \"{index}\" vector index?\n\nCollection: {collection}")
    }
    fn confirm_rollback_title(&self) -> &'static str {
        "Roll back script"
    }
    fn confirm_rollback(&self, name: &str, version: u32) -> String {
        format!("Roll the \"{name}\" script back to v{version}?")
    }
    fn rollback_button(&self) -> &'static str {
        "Roll back"
    }
    fn confirm_disable_auto_embed_title(&self) -> &'static str {
        "Disable auto-embed"
    }
    fn confirm_disable_auto_embed(&self, collection: &str) -> String {
        format!("Disable auto-embed on the \"{collection}\" collection?")
    }
    fn disable_button(&self) -> &'static str {
        "Disable"
    }
    fn confirm_clear_cache_title(&self) -> &'static str {
        "Clear embedding cache"
    }
    fn confirm_clear_cache(&self) -> &'static str {
        "Clear the whole embedding cache?"
    }
    fn clear_button(&self) -> &'static str {
        "Clear"
    }
    fn confirm_cancel_job_title(&self) -> &'static str {
        "Cancel embed job"
    }
    fn confirm_cancel_job(&self, job_id: &str) -> String {
        format!("Cancel the \"{job_id}\" embed job?")
    }
    fn cancel_job_button(&self) -> &'static str {
        "Cancel job"
    }
}
