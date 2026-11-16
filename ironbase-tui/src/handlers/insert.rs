//! Key handlers: insert

use crate::app::App;
use crate::widgets::Input;
use crossterm::event::{KeyCode, KeyModifiers};

/// Async insert key handler
pub(crate) async fn handle_insert_key_async(app: &mut App, key: KeyCode, modifiers: KeyModifiers) {
    match (key, modifiers) {
        (KeyCode::Esc, _) => app.close_modal(),
        (KeyCode::Char('s'), KeyModifiers::CONTROL) => {
            app.execute_insert_async().await;
        }
        // The editor takes the rest
        _ => {
            app.insert.handle_input(Input::Key(key, modifiers));
        }
    }
}
