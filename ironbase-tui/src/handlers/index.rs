//! Key handlers: index

use crate::app::App;
use crate::widgets::Input;
use crossterm::event::{KeyCode, KeyModifiers};

/// Async index key handler
pub(crate) async fn handle_index_key_async(app: &mut App, key: KeyCode, modifiers: KeyModifiers) {
    if app.index_state.is_creating {
        match (key, modifiers) {
            (KeyCode::Esc, _) => {
                app.index_state.cancel_create();
            }
            (KeyCode::Enter, _) => {
                app.execute_create_index_async().await;
            }
            (KeyCode::Tab, _) => {
                app.index_state.next_form_field();
            }
            (KeyCode::Char(' '), _) if app.index_state.form_field == 1 => {
                app.index_state.toggle_compound();
            }
            (KeyCode::Char(' '), _) if app.index_state.form_field == 2 => {
                app.index_state.toggle_unique();
            }
            (KeyCode::Char(' '), _) if app.index_state.form_field == 3 => {
                app.index_state.toggle_sparse();
            }
            _ if app.index_state.form_field == 0 => {
                app.index_state.handle_input(Input::Key(key, modifiers));
            }
            _ => {}
        }
    } else {
        match key {
            KeyCode::Esc => app.close_modal(),
            KeyCode::Up | KeyCode::Char('k') => app.index_state.select_up(),
            KeyCode::Down | KeyCode::Char('j') => app.index_state.select_down(),
            KeyCode::Tab | KeyCode::Char('n') => app.index_state.start_create(),
            KeyCode::Char('d') | KeyCode::Delete => app.open_drop_index_confirm(),
            KeyCode::Char('a') => app.execute_analyze_index_async().await,
            _ => {}
        }
    }
}
