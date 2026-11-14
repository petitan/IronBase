//! Key handlers: listener

use crate::app::App;
use crate::modals;
use crossterm::event::KeyCode;

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
                    match db.listener_add(&id, &bind, tls, None, None).await {
                        Ok(true) => {
                            app.listener_state
                                .set_success(format!("Listener '{}' added!", id));
                            app.listener_state.mode = ListenerModalMode::List;
                            app.listener_state.add_id.clear();
                            app.listener_state.add_bind.clear();

                            // Refresh listener list
                            if let Ok(listeners) = db.listener_list().await {
                                let infos: Vec<_> = listeners
                                    .iter()
                                    .filter_map(modals::listener::ListenerInfo::from_value)
                                    .collect();
                                app.listener_state.set_listeners(infos);
                            }
                        }
                        Ok(false) => {
                            app.listener_state
                                .set_error("Failed to add listener".to_string());
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
                        let result = match app.listener_state.confirm_action {
                            Some(ListenerAction::Delete) => db.listener_delete(&id).await,
                            Some(ListenerAction::Enable) => db.listener_enable(&id).await,
                            Some(ListenerAction::Disable) => db.listener_disable(&id).await,
                            None => Ok(false),
                        };

                        match result {
                            Ok(true) => {
                                let action = match app.listener_state.confirm_action {
                                    Some(ListenerAction::Delete) => "deleted",
                                    Some(ListenerAction::Enable) => "enabled",
                                    Some(ListenerAction::Disable) => "disabled",
                                    None => "modified",
                                };
                                app.listener_state
                                    .set_success(format!("Listener '{}' {}!", id, action));
                                app.listener_state.mode = ListenerModalMode::List;
                                app.listener_state.confirm_action = None;

                                // Refresh listener list
                                if let Ok(listeners) = db.listener_list().await {
                                    let infos: Vec<_> = listeners
                                        .iter()
                                        .filter_map(modals::listener::ListenerInfo::from_value)
                                        .collect();
                                    app.listener_state.set_listeners(infos);
                                }
                            }
                            Ok(false) => {
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
