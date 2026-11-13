//! Key handlers: confirm

use crate::app::App;
use crossterm::event::KeyCode;

/// Async confirm key handler
pub(crate) async fn handle_confirm_key_async(app: &mut App, key: KeyCode) {
    match key {
        KeyCode::Esc => app.close_confirm(),
        KeyCode::Left | KeyCode::Right | KeyCode::Tab | KeyCode::Char('h') | KeyCode::Char('l') => {
            app.confirm_toggle();
        }
        KeyCode::Enter => {
            app.execute_confirm_action_async().await;
        }
        KeyCode::Char('y') | KeyCode::Char('Y') => {
            app.confirm.selected = crate::modals::confirm::ConfirmOption::Confirm;
            app.execute_confirm_action_async().await;
        }
        KeyCode::Char('n') | KeyCode::Char('N') => {
            app.close_confirm();
        }
        _ => {}
    }
}
