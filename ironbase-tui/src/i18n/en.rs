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
}
