//! Key handlers: export

use crate::app::App;
use crate::widgets::Input;
use crossterm::event::{KeyCode, KeyModifiers};

/// Async export key handler
pub(crate) async fn handle_export_key_async(app: &mut App, key: KeyCode, modifiers: KeyModifiers) {
    match (key, modifiers) {
        (KeyCode::Esc, _) => app.close_modal(),
        (KeyCode::Enter, _) => {
            app.execute_export_async().await;
        }
        (KeyCode::Tab, _) => {
            if let Some(state) = app.export_state_mut() {
                state.toggle_format();
            }
        }
        _ => {
            if let Some(state) = app.export_state_mut() {
                state.handle_input(Input::Key(key, modifiers));
            }
        }
    }
}
