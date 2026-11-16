//! English texts

use super::Messages;

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
        "Detail"
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
        format!(" | Error: {error}")
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
            " [FILTERED – Esc clears]"
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
}
