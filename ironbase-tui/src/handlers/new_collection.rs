//! Key handlers: new_collection

use crate::app::App;
use crossterm::event::KeyCode;

/// Async new collection key handler
pub(crate) async fn handle_new_collection_key_async(app: &mut App, key: KeyCode) {
    match key {
        KeyCode::Esc => app.close_modal(),
        KeyCode::Enter => {
            app.create_collection_async().await;
        }
        KeyCode::Backspace => {
            app.new_collection_state.backspace();
        }
        KeyCode::Left => {
            app.new_collection_state.cursor_left();
        }
        KeyCode::Right => {
            app.new_collection_state.cursor_right();
        }
        KeyCode::Char(c) => {
            app.new_collection_state.insert_char(c);
        }
        _ => {}
    }
}
