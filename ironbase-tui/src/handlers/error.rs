//! Key handlers: error

use crate::app::App;
use crossterm::event::KeyCode;

/// Handle keys in error detail modal
pub(crate) fn handle_error_modal_key(app: &mut App, key: KeyCode) {
    match key {
        KeyCode::Esc | KeyCode::Enter | KeyCode::Char('q') => {
            app.close_modal();
            app.error_message = None;
            app.error_scroll = 0;
        }
        KeyCode::Up | KeyCode::Char('k') => {
            app.error_scroll = app.error_scroll.saturating_sub(1);
        }
        KeyCode::Down | KeyCode::Char('j') => {
            app.error_scroll = app.error_scroll.saturating_add(1);
        }
        KeyCode::PageUp => {
            app.error_scroll = app.error_scroll.saturating_sub(10);
        }
        KeyCode::PageDown => {
            app.error_scroll = app.error_scroll.saturating_add(10);
        }
        KeyCode::Home | KeyCode::Char('g') => {
            app.error_scroll = 0;
        }
        _ => {}
    }
}
