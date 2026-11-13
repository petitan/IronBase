//! Key handlers: actions

use super::database::handle_database_open;
use crate::app::App;
use crossterm::event::KeyCode;

/// Handle keys in actions modal (async for schema loading)
pub(crate) async fn handle_actions_key_async(app: &mut App, key: KeyCode) {
    match key {
        KeyCode::Esc | KeyCode::Char('q') => app.close_modal(),
        // Action shortcuts
        KeyCode::Char('i') => {
            app.close_modal();
            app.open_insert();
        }
        KeyCode::Char('e') => {
            app.close_modal();
            app.open_edit();
        }
        KeyCode::Char('d') => {
            app.close_modal();
            app.open_delete_confirm();
        }
        KeyCode::Char('x') => {
            app.close_modal();
            app.open_index_modal();
            app.load_indexes_async().await;
            app.load_index_statistics_async().await;
        }
        KeyCode::Char('r') => {
            app.close_modal();
            app.open_query_modal();
        }
        KeyCode::Char('f') => {
            app.close_modal();
            app.open_filter_modal_async().await;
        }
        KeyCode::Char('o') => {
            app.close_modal();
            app.open_export_modal();
        }
        KeyCode::Char('c') => {
            app.close_modal();
            app.open_new_collection();
        }
        KeyCode::Char('y') => {
            app.close_modal();
            app.copy_document_to_clipboard();
        }
        KeyCode::Char('D') => {
            app.close_modal();
            app.open_delete_collection_confirm();
        }
        KeyCode::Char('s') => {
            app.close_modal();
            app.open_script_modal();
            app.load_scripts_async().await;
        }
        KeyCode::Char('b') => {
            app.close_modal();
            handle_database_open(app);
        }
        _ => {}
    }
}
