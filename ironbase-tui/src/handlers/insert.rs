//! Key handlers: insert

use crate::app::App;
use crossterm::event::{KeyCode, KeyModifiers};

/// Async insert key handler
pub(crate) async fn handle_insert_key_async(app: &mut App, key: KeyCode, modifiers: KeyModifiers) {
    match (key, modifiers) {
        (KeyCode::Esc, _) => app.close_modal(),
        (KeyCode::Char('s'), KeyModifiers::CONTROL) => {
            app.execute_insert_async().await;
        }
        (KeyCode::Enter, KeyModifiers::NONE) => {
            app.insert.insert_newline();
        }
        (KeyCode::Tab, _) => {
            app.insert.insert_tab();
        }
        (KeyCode::Backspace, _) => {
            app.insert.backspace();
        }
        (KeyCode::Left, _) => {
            app.insert.cursor_left();
        }
        (KeyCode::Right, _) => {
            app.insert.cursor_right();
        }
        (KeyCode::Up, _) => {
            app.insert.cursor_up();
        }
        (KeyCode::Down, _) => {
            app.insert.cursor_down();
        }
        (KeyCode::Char(c), KeyModifiers::NONE) | (KeyCode::Char(c), KeyModifiers::SHIFT) => {
            app.insert.insert_char(c);
        }
        _ => {}
    }
}
