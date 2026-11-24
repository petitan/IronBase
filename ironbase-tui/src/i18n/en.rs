//! English texts

use super::{Messages, Op, ToolCategory};
use crate::state::filter::{FilterOperator, SortDirection};
use crate::state::query::QueryTemplate;
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

    fn filter_title(&self) -> &'static str {
        "Filter"
    }
    fn key_choose_field(&self) -> &'static str {
        "Choose field"
    }
    fn key_next(&self) -> &'static str {
        "Next"
    }
    fn field_title(&self, count: usize) -> String {
        match count {
            0 => " Field ".to_string(),
            1 => " Field (1 field) [↑↓] ".to_string(),
            _ => format!(" Field ({count} fields) [↑↓] "),
        }
    }
    fn field_placeholder_examples(&self) -> &'static str {
        "e.g. name, age, email..."
    }
    fn field_placeholder_pick(&self) -> &'static str {
        "type, or pick with the arrows..."
    }
    fn label_operator(&self) -> &'static str {
        "Operator"
    }
    fn label_value(&self) -> &'static str {
        "Value"
    }
    fn value_placeholder(&self) -> &'static str {
        "value to search for..."
    }
    fn hint_type_field(&self) -> &'static str {
        "Type a field name (e.g. name, age, email)"
    }
    fn hint_field_next(&self) -> &'static str {
        "Enter/Tab: on to the operator"
    }
    fn hint_operator(&self) -> &'static str {
        "←/→: choose the operator, Enter/Tab: next"
    }
    fn hint_add_exists(&self) -> &'static str {
        "Enter: add the condition (no value needed)"
    }
    fn hint_type_value(&self) -> &'static str {
        "Type the value to search for, or ^S: search now"
    }
    fn hint_add_condition(&self) -> &'static str {
        "Enter: add the condition | ^S: search now"
    }
    fn hint_filters(&self) -> &'static str {
        "Enter: edit | Del: remove | Ctrl+↑↓: move | ^S: search"
    }
    fn hint_sort(&self) -> &'static str {
        "←/→: choose the field | Space: direction | ^S: search"
    }
    fn active_filters_title(&self, count: usize) -> String {
        format!(" Active filters ({count}) ")
    }
    fn no_active_filters(&self) -> &'static str {
        "No active filter – add one!"
    }
    fn label_sort(&self) -> &'static str {
        "Sort"
    }
    fn no_sort(&self) -> &'static str {
        "No sort"
    }
    fn press_ctrl_s_to_search(&self) -> &'static str {
        "Press ^S to search"
    }
    fn add_filters(&self) -> &'static str {
        "Add filters"
    }
    fn query_title(&self) -> &'static str {
        "Query builder"
    }
    fn key_run(&self) -> &'static str {
        "Run"
    }
    fn key_plan(&self) -> &'static str {
        "Plan"
    }
    fn key_templates(&self) -> &'static str {
        "Templates"
    }
    fn query_results_header(&self, count: usize, total: usize) -> String {
        if total > count {
            format!("Results ({count}/{total}) [^j/k] [Enter: filter]:")
        } else {
            format!("Results ({count}) [^j/k] [Enter: filter]:")
        }
    }
    fn label_result(&self) -> &'static str {
        "Result"
    }
    fn query_prompt(&self) -> &'static str {
        "Write a query and press ^S (Ctrl+S)"
    }
    fn query_status(&self, line: usize, column: usize, collection: &str, plan: bool) -> String {
        {
            let plan = if plan { " [plan]" } else { "" };
            format!("Line: {line} Column: {column} | Collection: {collection}{plan}")
        }
    }
    fn query_plan_title(&self) -> &'static str {
        "Query plan"
    }
    fn label_plan(&self) -> &'static str {
        "Plan"
    }
    fn label_index(&self) -> &'static str {
        "Index"
    }
    fn label_estimated_rows(&self) -> &'static str {
        "Est. rows"
    }
    fn label_stage(&self) -> &'static str {
        "Stage"
    }
    fn label_cost(&self) -> &'static str {
        "Cost"
    }
    fn label_reason(&self) -> &'static str {
        "Reason"
    }
    fn no_query_plan(&self) -> &'static str {
        "No query plan yet. Press ^E."
    }
    fn query_plan_toggle_hint(&self) -> &'static str {
        "[^E] Switch between the results and the plan"
    }
    fn server_info_title(&self) -> &'static str {
        "Server info [j/k: scroll, Esc: close]"
    }
    fn loading(&self) -> &'static str {
        "Loading..."
    }
    fn database_info(&self) -> &'static str {
        "Database information"
    }
    fn label_server(&self) -> &'static str {
        "Server"
    }
    fn label_api_key(&self) -> &'static str {
        "API key"
    }
    fn api_key_required(&self) -> &'static str {
        "Required"
    }
    fn api_key_not_required(&self) -> &'static str {
        "Not required"
    }
    fn label_database(&self) -> &'static str {
        "Database"
    }
    fn label_file_size(&self) -> &'static str {
        "File size"
    }
    fn label_collection_count(&self) -> &'static str {
        "Collections"
    }
    fn label_total_documents(&self) -> &'static str {
        "Total documents"
    }
    fn label_total_indexes(&self) -> &'static str {
        "Total indexes"
    }
    fn label_of_which_vector(&self) -> &'static str {
        "of which vector"
    }
    fn vector_index_summary(&self, indexes: usize, vectors: usize) -> String {
        {
            let label = if vectors == 1 { "vector" } else { "vectors" };
            format!("{indexes} ({vectors} {label})")
        }
    }
    fn label_collections(&self) -> &'static str {
        "Collections"
    }
    fn available_tools(&self, count: usize) -> String {
        format!("Available MCP tools ({count})")
    }
    fn available_prompts(&self, count: usize) -> String {
        format!("Available MCP prompts ({count})")
    }
    fn no_description(&self) -> &'static str {
        "No description"
    }
    fn protocol_version(&self, version: &str) -> String {
        format!("MCP protocol version: {version}")
    }

    fn filter_operator(&self, operator: FilterOperator) -> &'static str {
        match operator {
            FilterOperator::Equals => "=",
            FilterOperator::NotEquals => "≠",
            FilterOperator::GreaterThan => ">",
            FilterOperator::LessThan => "<",
            FilterOperator::GreaterOrEq => ">=",
            FilterOperator::LessOrEq => "<=",
            FilterOperator::Contains => "contains",
            FilterOperator::StartsWith => "starts with",
            FilterOperator::Exists => "exists",
            FilterOperator::In => "in list",
        }
    }
    fn sort_direction(&self, direction: SortDirection) -> &'static str {
        match direction {
            SortDirection::Asc => "↑ ascending",
            SortDirection::Desc => "↓ descending",
        }
    }
    fn query_template(&self, template: QueryTemplate) -> &'static str {
        match template {
            QueryTemplate::All => "All",
            QueryTemplate::ById => "By _id",
            QueryTemplate::Equals => "Equality",
            QueryTemplate::GreaterThan => "Greater than",
            QueryTemplate::LessThan => "Less than",
            QueryTemplate::In => "In a list",
            QueryTemplate::Exists => "Exists",
            QueryTemplate::Regex => "Regex",
            QueryTemplate::And => "AND",
            QueryTemplate::Or => "OR",
        }
    }
    fn tool_category(&self, category: ToolCategory) -> &'static str {
        match category {
            ToolCategory::Database => "Database",
            ToolCategory::Collections => "Collections",
            ToolCategory::Query => "Query",
            ToolCategory::Insert => "Insert",
            ToolCategory::Update => "Update",
            ToolCategory::Delete => "Delete",
            ToolCategory::Count => "Count",
            ToolCategory::Aggregate => "Aggregation",
            ToolCategory::Indexes => "Indexes",
            ToolCategory::Schema => "Schema",
            ToolCategory::Scripts => "Scripts",
            ToolCategory::Fuzzy => "Fuzzy search",
            ToolCategory::Other => "Other",
        }
    }

    fn no(&self) -> &'static str {
        "No"
    }
    fn key_new(&self) -> &'static str {
        "New"
    }
    fn key_tabs(&self) -> &'static str {
        "Tabs"
    }
    fn key_next_field(&self) -> &'static str {
        "Next field"
    }
    fn label_status(&self) -> &'static str {
        "Status"
    }
    fn label_field_name(&self) -> &'static str {
        "Field name"
    }
    fn status_active(&self) -> &'static str {
        "Active"
    }
    fn api_keys_title(&self) -> &'static str {
        "API keys"
    }
    fn key_revoke(&self) -> &'static str {
        "Revoke"
    }
    fn admin_needs_localhost(&self) -> &'static str {
        "(Admin operations need a localhost connection)"
    }
    fn admin_needs_key(&self) -> &'static str {
        "(Set IRONBASE_ADMIN_KEY for admin operations)"
    }
    fn new_key_label(&self) -> &'static str {
        "New key"
    }
    fn new_key_warning(&self) -> &'static str {
        "(copy it now, it won't be shown again!)"
    }
    fn no_api_keys(&self) -> &'static str {
        "No API keys"
    }
    fn label_key(&self) -> &'static str {
        "Key"
    }
    fn status_revoked(&self) -> &'static str {
        "Revoked"
    }
    fn enter_new_key_name(&self) -> &'static str {
        "Enter a name for the new API key:"
    }
    fn confirm_revoke_key(&self, name: &str) -> String {
        format!("Revoke the API key \"{name}\"?")
    }
    fn confirm_delete_key(&self, name: &str) -> String {
        format!("DELETE the API key \"{name}\"?")
    }
    fn listeners_title(&self) -> &'static str {
        "Listeners"
    }
    fn key_enable(&self) -> &'static str {
        "Enable"
    }
    fn key_disable(&self) -> &'static str {
        "Disable"
    }
    fn restart_needed(&self) -> &'static str {
        "Changes take effect after a server restart"
    }
    fn no_listeners(&self) -> &'static str {
        "No listeners configured"
    }
    fn label_bind(&self) -> &'static str {
        "Bind"
    }
    fn status_disabled(&self) -> &'static str {
        "Disabled"
    }
    fn enter_listener_details(&self) -> &'static str {
        "Enter the listener details (Tab: next field):"
    }
    fn label_bind_address(&self) -> &'static str {
        "Bind address"
    }
    fn checkbox_enabled(&self) -> &'static str {
        "[x] Enabled"
    }
    fn checkbox_disabled(&self) -> &'static str {
        "[ ] Disabled"
    }
    fn space_to_toggle(&self) -> &'static str {
        "(Space: toggle)"
    }
    fn confirm_delete_listener(&self, id: &str) -> String {
        format!("DELETE the listener \"{id}\"?")
    }
    fn confirm_enable_listener(&self, id: &str) -> String {
        format!("Enable the listener \"{id}\"?")
    }
    fn confirm_disable_listener(&self, id: &str) -> String {
        format!("Disable the listener \"{id}\"?")
    }
    fn listener_read_only(&self) -> &'static str {
        "Read-only: managing listeners needs a localhost connection"
    }
    fn vector_search_title(&self) -> &'static str {
        "Vector search"
    }
    fn tab_create_index(&self) -> &'static str {
        "Create index"
    }
    fn tab_list_indexes(&self) -> &'static str {
        "Index list"
    }
    fn label_k_limit(&self) -> &'static str {
        "K (limit)"
    }
    fn label_vector_json(&self) -> &'static str {
        "Vector (JSON array)"
    }
    fn label_filter_optional(&self) -> &'static str {
        "Filter (optional JSON)"
    }
    fn vector_results_title(&self, count: usize, loading: bool) -> String {
        if loading {
            " Results (loading...) ".to_string()
        } else {
            format!(" Results ({count}) ")
        }
    }
    fn no_vector_results(&self) -> &'static str {
        "No results. Enter a vector and press Enter to search."
    }
    fn key_toggle_metric(&self) -> &'static str {
        "Toggle metric"
    }
    fn label_dimension(&self) -> &'static str {
        "Dimension (0 = auto)"
    }
    fn label_distance_metric(&self) -> &'static str {
        "Distance metric [Space: change]"
    }
    fn metric_cosine(&self) -> &'static str {
        "Cosine (recommended for embeddings)"
    }
    fn metric_euclidean(&self) -> &'static str {
        "Euclidean (L2 distance)"
    }
    fn metric_dot_product(&self) -> &'static str {
        "Dot product (for normalized vectors)"
    }
    fn dimension_tip(&self) -> &'static str {
        "Tip: common embeddings have 384, 768 or 1536 dimensions."
    }
    fn vector_indexes_title(&self, count: usize) -> String {
        format!(" Vector indexes ({count}) ")
    }
    fn no_vector_indexes(&self) -> &'static str {
        "No vector indexes. Create one on the \"Create index\" tab."
    }
    fn vector_index_row(
        &self,
        name: &str,
        field: &str,
        dimension: usize,
        metric: &str,
        vectors: usize,
    ) -> String {
        {
            let label = if vectors == 1 { "vector" } else { "vectors" };
            format!("{name} | {field} | dim:{dimension} | {metric} | {vectors} {label}")
        }
    }
    fn index_title(&self) -> &'static str {
        "Index management"
    }
    fn key_analyze(&self) -> &'static str {
        "Analyze"
    }
    fn existing_indexes_title(&self, analyzing: bool) -> String {
        if analyzing {
            " Existing indexes (analyzing...) ".to_string()
        } else {
            " Existing indexes ".to_string()
        }
    }
    fn no_indexes_only_id(&self) -> &'static str {
        "No index (only _id)"
    }
    fn index_statistics_title(&self) -> &'static str {
        "Index statistics"
    }
    fn no_index_selected(&self) -> &'static str {
        "No index selected"
    }
    fn load_statistics_hint(&self) -> &'static str {
        "Load the statistics: [a] Analyze"
    }
    fn staleness_partial(&self) -> &'static str {
        "Partial"
    }
    fn staleness_stale(&self) -> &'static str {
        "Stale"
    }
    fn label_keys(&self) -> &'static str {
        "Keys"
    }
    fn label_distinct(&self) -> &'static str {
        "Distinct"
    }
    fn label_histogram(&self) -> &'static str {
        "Histogram"
    }
    fn create_index_title(&self) -> &'static str {
        "Create a new index"
    }
    fn label_fields_comma(&self) -> &'static str {
        "Fields (comma separated)"
    }
    fn label_compound(&self) -> &'static str {
        "Compound"
    }
    fn compound_field_count(&self, count: usize) -> String {
        match count {
            1 => " [1 field]".to_string(),
            _ => format!(" [{count} fields]"),
        }
    }
    fn label_unique(&self) -> &'static str {
        "Unique"
    }
    fn label_sparse(&self) -> &'static str {
        "Sparse"
    }
    fn sparse_hint(&self) -> &'static str {
        "(only the documents that have the field)"
    }
    fn stale_indexes(&self, count: usize) -> String {
        match count {
            1 => "1 stale index – [a] Analyze".to_string(),
            _ => format!("{count} stale indexes – [a] Analyze"),
        }
    }
}
