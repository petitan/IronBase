//! Key handlers: query

use crate::app::App;
use crossterm::event::{KeyCode, KeyModifiers};

/// Async query key handler
pub(crate) async fn handle_query_key_async(app: &mut App, key: KeyCode, modifiers: KeyModifiers) {
    // Handle template mode separately
    if app.query_state.show_templates {
        match key {
            KeyCode::Esc => app.query_state.show_templates = false,
            KeyCode::Tab => app.query_state.show_templates = false,
            KeyCode::Up | KeyCode::Char('k') => app.query_state.template_up(),
            KeyCode::Down | KeyCode::Char('j') => app.query_state.template_down(),
            KeyCode::Enter => app.query_state.apply_template(),
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
            if app.query_state.explain_result.is_some() {
                // Toggle between results and explain view
                app.query_state.toggle_explain_view();
            } else {
                // Execute explain
                app.execute_explain_async().await;
            }
        }
        // Enter = Apply filter (ha van eredmény) VAGY newline (ha nincs)
        (KeyCode::Enter, KeyModifiers::NONE) => {
            if app.query_state.has_results() {
                app.apply_query_as_filter().await;
            } else {
                app.query_state.insert_newline();
            }
        }
        // Tab = Insert 2 spaces
        (KeyCode::Tab, _) => {
            app.query_state.insert_tab();
        }
        // Ctrl+T = Templates
        (KeyCode::Char('t'), KeyModifiers::CONTROL) => {
            app.query_state.toggle_templates();
        }
        // Ctrl+j/k = Navigate results
        (KeyCode::Char('j'), KeyModifiers::CONTROL) => {
            app.query_state.result_down();
        }
        (KeyCode::Char('k'), KeyModifiers::CONTROL) => {
            app.query_state.result_up();
        }
        // Arrow keys = cursor movement
        (KeyCode::Backspace, _) => {
            app.query_state.backspace();
        }
        (KeyCode::Left, _) => {
            app.query_state.cursor_left();
        }
        (KeyCode::Right, _) => {
            app.query_state.cursor_right();
        }
        (KeyCode::Up, _) => {
            app.query_state.cursor_up();
        }
        (KeyCode::Down, _) => {
            app.query_state.cursor_down();
        }
        // All other chars = insert
        (KeyCode::Char(c), KeyModifiers::NONE) | (KeyCode::Char(c), KeyModifiers::SHIFT) => {
            app.query_state.insert_char(c);
        }
        _ => {}
    }
}
