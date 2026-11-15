//! Key handlers: listener

use crate::app::App;
use crate::db::DbWrapper;
use crossterm::event::KeyCode;
use ironbase_mcp_types::params::{ListenerAddParams, ListenerIdParams, NoParams};
use ironbase_mcp_types::responses::{ListenerAction as Created, ListenerInfo};
use ironbase_mcp_types::tools::{
    ListenerCreate, ListenerDelete, ListenerDisable, ListenerEnable, ListenerList,
};

/// The server's listeners (`listener_list`)
pub(crate) async fn load_listeners(db: &DbWrapper) -> anyhow::Result<Vec<ListenerInfo>> {
    Ok(db.call::<ListenerList>(&NoParams {}).await?.listeners)
}

/// Reload the listener list after a change; a failed reload is reported
async fn refresh_listeners(app: &mut App) {
    let Some(ref db) = app.db else { return };
    match load_listeners(db).await {
        Ok(listeners) => app.listener_state.set_listeners(listeners),
        Err(e) => app
            .listener_state
            .set_error(format!("Failed to reload listeners: {}", e)),
    }
}

/// Listener modal key handler
pub(crate) async fn handle_listener_key_async(app: &mut App, key: KeyCode) {
    use crate::modals::listener::{AddField, ListenerAction, ListenerModalMode};

    match app.listener_state.mode {
        ListenerModalMode::List => match key {
            KeyCode::Esc => {
                app.close_modal();
            }
            KeyCode::Down | KeyCode::Char('j') => {
                app.listener_state.select_next();
            }
            KeyCode::Up | KeyCode::Char('k') => {
                app.listener_state.select_prev();
            }
            KeyCode::Char('n') if app.listener_state.is_localhost => {
                app.listener_state.start_add();
            }
            KeyCode::Char('e') if app.listener_state.is_localhost => {
                app.listener_state.start_enable();
            }
            KeyCode::Char('x') if app.listener_state.is_localhost => {
                app.listener_state.start_disable();
            }
            KeyCode::Char('d') if app.listener_state.is_localhost => {
                app.listener_state.start_delete();
            }
            _ => {}
        },
        ListenerModalMode::Add => match key {
            KeyCode::Esc => {
                app.listener_state.cancel();
            }
            KeyCode::Tab => {
                app.listener_state.next_add_field();
            }
            KeyCode::Char(' ') if app.listener_state.add_field == AddField::Tls => {
                app.listener_state.toggle_tls();
            }
            KeyCode::Enter => {
                let id = app.listener_state.add_id.trim().to_string();
                let bind = app.listener_state.add_bind.trim().to_string();
                let tls = app.listener_state.add_tls;

                if id.is_empty() {
                    app.listener_state
                        .set_error("ID cannot be empty".to_string());
                    return;
                }
                if bind.is_empty() {
                    app.listener_state
                        .set_error("Bind address cannot be empty".to_string());
                    return;
                }

                if let Some(ref db) = app.db {
                    let args = ListenerAddParams {
                        id: id.clone(),
                        bind,
                        tls,
                        cert_path: None,
                        key_path: None,
                        description: None,
                    };
                    match db.call::<ListenerCreate>(&args).await {
                        Ok(response) => {
                            // Replacing an existing listener is not "added"; the
                            // server applies either on its next start
                            let action = match response.action {
                                Created::Created => "added",
                                Created::Updated => "replaced",
                            };
                            app.listener_state.set_success(format!(
                                "Listener '{}' {}. {}",
                                id, action, response.note
                            ));
                            app.listener_state.mode = ListenerModalMode::List;
                            app.listener_state.add_id.clear();
                            app.listener_state.add_bind.clear();
                            refresh_listeners(app).await;
                        }
                        Err(e) => {
                            app.listener_state.set_error(format!("Failed: {}", e));
                        }
                    }
                }
            }
            KeyCode::Char(c) => match app.listener_state.add_field {
                AddField::Id => app.listener_state.add_id.push(c),
                AddField::Bind => app.listener_state.add_bind.push(c),
                AddField::Tls => {}
            },
            KeyCode::Backspace => match app.listener_state.add_field {
                AddField::Id => {
                    app.listener_state.add_id.pop();
                }
                AddField::Bind => {
                    app.listener_state.add_bind.pop();
                }
                AddField::Tls => {}
            },
            _ => {}
        },
        ListenerModalMode::Confirm => match key {
            KeyCode::Esc | KeyCode::Char('n') => {
                app.listener_state.cancel();
            }
            KeyCode::Char('y') => {
                if let Some(listener) = app.listener_state.get_selected() {
                    let id = listener.id.clone();

                    if let Some(ref db) = app.db {
                        let args = ListenerIdParams { id: id.clone() };
                        // (found, the server's note)
                        let result = match app.listener_state.confirm_action {
                            Some(ListenerAction::Delete) => db
                                .call::<ListenerDelete>(&args)
                                .await
                                .map(|r| (r.deleted, r.note)),
                            Some(ListenerAction::Enable) => db
                                .call::<ListenerEnable>(&args)
                                .await
                                .map(|r| (r.enabled, r.note)),
                            Some(ListenerAction::Disable) => db
                                .call::<ListenerDisable>(&args)
                                .await
                                .map(|r| (r.disabled, r.note)),
                            None => Ok((false, String::new())),
                        };

                        match result {
                            Ok((true, note)) => {
                                // The note says what changed and that it waits
                                // for a restart
                                app.listener_state
                                    .set_success(format!("'{}': {}", id, note));
                                app.listener_state.mode = ListenerModalMode::List;
                                app.listener_state.confirm_action = None;
                                refresh_listeners(app).await;
                            }
                            Ok((false, _)) => {
                                app.listener_state
                                    .set_error("Listener not found".to_string());
                                app.listener_state.cancel();
                            }
                            Err(e) => {
                                app.listener_state.set_error(format!("Failed: {}", e));
                                app.listener_state.cancel();
                            }
                        }
                    }
                }
            }
            _ => {}
        },
    }
}
