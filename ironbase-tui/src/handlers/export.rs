//! Key handlers: export

use crate::app::App;
use crossterm::event::{KeyCode, KeyModifiers};

/// Async export key handler
pub(crate) async fn handle_export_key_async(app: &mut App, key: KeyCode, modifiers: KeyModifiers) {
    match (key, modifiers) {
        (KeyCode::Esc, _) => app.close_modal(),
        (KeyCode::Enter, _) => {
            app.execute_export_async().await;
        }
        (KeyCode::Tab, _) => {
            app.export_state.toggle_format();
        }
        _ => {
            app.export_state.handle_input(key, modifiers);
        }
    }
}
