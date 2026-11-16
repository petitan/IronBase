//! Key handlers: new_collection

use crate::app::App;
use crate::widgets::Input;
use crossterm::event::{KeyCode, KeyModifiers};

/// Async new collection key handler
pub(crate) async fn handle_new_collection_key_async(
    app: &mut App,
    key: KeyCode,
    modifiers: KeyModifiers,
) {
    match key {
        KeyCode::Esc => app.close_modal(),
        KeyCode::Enter => {
            app.create_collection_async().await;
        }
        _ => {
            app.new_collection_state
                .handle_input(Input::Key(key, modifiers));
        }
    }
}
