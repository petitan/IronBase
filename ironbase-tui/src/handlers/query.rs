//! Key handlers: query

use crate::app::App;
use crate::state::open_state;
use crate::widgets::Input;
use crossterm::event::{KeyCode, KeyModifiers};

/// Async query key handler
pub(crate) async fn handle_query_key_async(app: &mut App, key: KeyCode, modifiers: KeyModifiers) {
    // Handle template mode separately
    if open_state!(app, Query).show_templates {
        match key {
            KeyCode::Esc => open_state!(app, Query).show_templates = false,
            KeyCode::Tab => open_state!(app, Query).show_templates = false,
            KeyCode::Up | KeyCode::Char('k') => open_state!(app, Query).template_up(),
            KeyCode::Down | KeyCode::Char('j') => open_state!(app, Query).template_down(),
            KeyCode::Enter => open_state!(app, Query).apply_template(),
            _ => {}
        }
        return;
    }

    match (key, modifiers) {
        (KeyCode::Esc, _) => app.close_modal(),
        // Ctrl+S = Run query
        (KeyCode::Char('s'), KeyModifiers::CONTROL) => {
            app.execute_query_async().await;
        }
        // Ctrl+E = Explain query OR toggle explain view
        (KeyCode::Char('e'), KeyModifiers::CONTROL) => {
            if open_state!(app, Query).explain_result.is_some() {
                // Toggle between results and explain view
                open_state!(app, Query).toggle_explain_view();
            } else {
                // Execute explain
                app.execute_explain_async().await;
            }
        }
        // Enter = Apply filter (ha van eredmény) VAGY newline (ha nincs)
        (KeyCode::Enter, KeyModifiers::NONE) => {
            if open_state!(app, Query).has_results() {
                app.apply_query_as_filter().await;
            } else {
                open_state!(app, Query).handle_input(Input::Key(key, modifiers));
            }
        }
        // Ctrl+T = Templates
        (KeyCode::Char('t'), KeyModifiers::CONTROL) => {
            open_state!(app, Query).toggle_templates();
        }
        // Ctrl+j/k = Navigate results
        (KeyCode::Char('j'), KeyModifiers::CONTROL) => {
            open_state!(app, Query).result_down();
        }
        (KeyCode::Char('k'), KeyModifiers::CONTROL) => {
            open_state!(app, Query).result_up();
        }
        // The editor takes the rest
        _ => {
            open_state!(app, Query).handle_input(Input::Key(key, modifiers));
        }
    }
}
