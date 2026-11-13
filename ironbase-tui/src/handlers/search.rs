//! Key handlers: search

use crate::app::App;
use crossterm::event::{KeyCode, KeyModifiers};

/// Async search key handler
pub(crate) async fn handle_search_key_async(app: &mut App, key: KeyCode, _modifiers: KeyModifiers) {
    use crate::app::SearchMode;

    match app.search.mode {
        SearchMode::Collections => handle_collection_search_key(app, key).await,
        SearchMode::Document => handle_document_search_key(app, key),
    }
}

/// Handle collection search keys
pub(crate) async fn handle_collection_search_key(app: &mut App, key: KeyCode) {
    if app.search.input_active {
        match key {
            KeyCode::Esc => app.close_modal(),
            KeyCode::Enter => {
                if app.search.query_input.is_empty() {
                    app.close_modal();
                } else {
                    app.execute_search_async().await;
                    if !app.search.results.is_empty() {
                        app.search.input_active = false;
                    }
                }
            }
            KeyCode::Char(c) => app.search.insert_char(c),
            KeyCode::Backspace => app.search.delete_char(),
            KeyCode::Left => app.search.cursor_left(),
            KeyCode::Right => app.search.cursor_right(),
            KeyCode::Tab | KeyCode::Down if !app.search.results.is_empty() => {
                app.search.input_active = false;
            }
            _ => {}
        }
    } else {
        match key {
            KeyCode::Esc => app.close_modal(),
            KeyCode::Enter => app.goto_search_result_async().await,
            KeyCode::Up | KeyCode::Char('k') => {
                if app.search.selected_result == 0 {
                    app.search.input_active = true;
                } else {
                    app.search_select_up();
                }
            }
            KeyCode::Down | KeyCode::Char('j') => app.search_select_down(),
            KeyCode::Tab | KeyCode::Char('/') => app.search.input_active = true,
            _ => {}
        }
    }
}

/// Handle document search keys
pub(crate) fn handle_document_search_key(app: &mut App, key: KeyCode) {
    match key {
        KeyCode::Esc => app.close_modal(),
        KeyCode::Enter if !app.search.query_input.is_empty() => {
            app.execute_document_search();
            // Ha van találat, zárd be a modalt és váltsd Detail pane-re
            if !app.search.doc_matches.is_empty() {
                app.close_modal();
                app.active_pane = crate::app::Pane::Detail;
            }
        }
        // n/N are typed like any other character here; match navigation (n/N)
        // lives in the Detail pane, where the modal lands after a successful search
        KeyCode::Char(c) => app.search.insert_char(c),
        KeyCode::Backspace => app.search.delete_char(),
        KeyCode::Left => app.search.cursor_left(),
        KeyCode::Right => app.search.cursor_right(),
        _ => {}
    }
}
