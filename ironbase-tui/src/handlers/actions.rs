//! Key handlers: actions

use super::global::run_action;
use crate::app::App;
use crate::keymap;
use crossterm::event::KeyCode;

/// Handle keys in the actions menu: its key map (`keymap::ACTIONS_MENU`)
/// names the action, the main screen's `run_action` does it
pub(crate) async fn handle_actions_key_async(app: &mut App, key: KeyCode) {
    match key {
        KeyCode::Esc | KeyCode::Char('q') => app.close_modal(),
        _ => {
            if let Some(action) = keymap::menu_action(key) {
                app.close_modal();
                run_action(app, action).await;
            }
        }
    }
}
