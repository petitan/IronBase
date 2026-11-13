//! Key handlers: help

use crate::app::App;
use crate::modals;
use crossterm::event::KeyCode;

/// Help modal key handler with scrolling support
pub(crate) fn handle_help_key(app: &mut App, key: KeyCode) {
    match key {
        KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('?') => {
            app.help_scroll = 0;
            app.close_modal();
        }
        KeyCode::Down | KeyCode::Char('j')
            if app.help_scroll < modals::help::HELP_LINES.saturating_sub(10) =>
        {
            app.help_scroll += 1;
        }
        KeyCode::Up | KeyCode::Char('k') => {
            app.help_scroll = app.help_scroll.saturating_sub(1);
        }
        KeyCode::PageDown => {
            app.help_scroll =
                (app.help_scroll + 5).min(modals::help::HELP_LINES.saturating_sub(10));
        }
        KeyCode::PageUp => {
            app.help_scroll = app.help_scroll.saturating_sub(5);
        }
        KeyCode::Home | KeyCode::Char('g') => {
            app.help_scroll = 0;
        }
        KeyCode::End | KeyCode::Char('G') => {
            app.help_scroll = modals::help::HELP_LINES.saturating_sub(10);
        }
        _ => {}
    }
}
