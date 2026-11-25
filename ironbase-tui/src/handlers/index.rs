//! Key handlers: index

use crate::app::App;
use crate::state::open_state;
use crate::widgets::Input;
use crossterm::event::{KeyCode, KeyModifiers};

/// Async index key handler
pub(crate) async fn handle_index_key_async(app: &mut App, key: KeyCode, modifiers: KeyModifiers) {
    if open_state!(app, Index).is_creating {
        match (key, modifiers) {
            (KeyCode::Esc, _) => {
                open_state!(app, Index).cancel_create();
            }
            (KeyCode::Enter, _) => {
                app.execute_create_index();
            }
            (KeyCode::Tab, _) => {
                open_state!(app, Index).next_form_field();
            }
            (KeyCode::Char(' '), _) if open_state!(app, Index).form_field == 1 => {
                open_state!(app, Index).toggle_compound();
            }
            (KeyCode::Char(' '), _) if open_state!(app, Index).form_field == 2 => {
                open_state!(app, Index).toggle_unique();
            }
            (KeyCode::Char(' '), _) if open_state!(app, Index).form_field == 3 => {
                open_state!(app, Index).toggle_sparse();
            }
            _ if open_state!(app, Index).form_field == 0 => {
                open_state!(app, Index).handle_input(Input::Key(key, modifiers));
            }
            _ => {}
        }
    } else {
        match key {
            KeyCode::Esc => app.close_modal(),
            KeyCode::Up | KeyCode::Char('k') => open_state!(app, Index).select_up(),
            KeyCode::Down | KeyCode::Char('j') => open_state!(app, Index).select_down(),
            KeyCode::Tab | KeyCode::Char('n') => open_state!(app, Index).start_create(),
            KeyCode::Char('d') | KeyCode::Delete => app.open_drop_index_confirm(),
            KeyCode::Char('a') => app.execute_analyze_index_async().await,
            _ => {}
        }
    }
}
