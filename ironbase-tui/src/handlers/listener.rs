//! Key handlers: listener

use crate::app::App;
use crate::db::DbWrapper;
use crate::i18n::{t, Op};
use crate::state::open_state;
use crate::widgets::Input;
use crossterm::event::{KeyCode, KeyModifiers};
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
/// next to the change's own outcome (which it does not hide)
async fn refresh_listeners(app: &mut App) {
    let Some(ref db) = app.db else { return };
    match load_listeners(db).await {
        Ok(listeners) => open_state!(app, Listener).set_listeners(listeners),
        Err(e) => {
            let state = &mut open_state!(app, Listener);
            let outcome = state.success.take().or(state.error.take());
            let reload = t().failed(Op::ReloadList, &e);
            state.set_error(match outcome {
                Some(outcome) => format!("{} ({})", outcome, reload),
                None => reload,
            });
        }
    }
}

/// Listener modal key handler
pub(crate) async fn handle_listener_key_async(
    app: &mut App,
    key: KeyCode,
    modifiers: KeyModifiers,
) {
    use crate::modals::listener::{AddField, ListenerAction, ListenerModalMode};

    match open_state!(app, Listener).mode {
        ListenerModalMode::List => match key {
            KeyCode::Esc => {
                app.close_modal();
            }
            KeyCode::Down | KeyCode::Char('j') => {
                open_state!(app, Listener).select_next();
            }
            KeyCode::Up | KeyCode::Char('k') => {
                open_state!(app, Listener).select_prev();
            }
            KeyCode::Char('n') if open_state!(app, Listener).is_localhost => {
                open_state!(app, Listener).start_add();
            }
            KeyCode::Char('e') if open_state!(app, Listener).is_localhost => {
                open_state!(app, Listener).start_enable();
            }
            KeyCode::Char('x') if open_state!(app, Listener).is_localhost => {
                open_state!(app, Listener).start_disable();
            }
            KeyCode::Char('d') if open_state!(app, Listener).is_localhost => {
                open_state!(app, Listener).start_delete();
            }
            _ => {}
        },
        ListenerModalMode::Add => match key {
            KeyCode::Esc => {
                open_state!(app, Listener).cancel();
            }
            KeyCode::Tab => {
                open_state!(app, Listener).next_add_field();
            }
            KeyCode::Char(' ') if open_state!(app, Listener).add_field == AddField::Tls => {
                open_state!(app, Listener).toggle_tls();
            }
            KeyCode::Enter => {
                let id = open_state!(app, Listener).add_id.value().trim().to_string();
                let bind = open_state!(app, Listener)
                    .add_bind
                    .value()
                    .trim()
                    .to_string();
                let tls = open_state!(app, Listener).add_tls;

                if id.is_empty() {
                    open_state!(app, Listener).set_error(t().id_required().to_string());
                    return;
                }
                if bind.is_empty() {
                    open_state!(app, Listener).set_error(t().bind_required().to_string());
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
                            let replaced = matches!(response.action, Created::Updated);
                            open_state!(app, Listener).set_success(t().listener_created(
                                &id,
                                replaced,
                                &response.note,
                            ));
                            open_state!(app, Listener).mode = ListenerModalMode::List;
                            open_state!(app, Listener).add_id.clear();
                            open_state!(app, Listener).add_bind.clear();
                            refresh_listeners(app).await;
                        }
                        Err(e) => {
                            open_state!(app, Listener).set_error(t().failed(Op::Create, &e));
                        }
                    }
                }
            }
            _ => {
                open_state!(app, Listener).handle_input(Input::Key(key, modifiers));
            }
        },
        ListenerModalMode::Confirm => match key {
            KeyCode::Esc | KeyCode::Char('n') => {
                open_state!(app, Listener).cancel();
            }
            KeyCode::Char('y') => {
                if let Some(listener) = open_state!(app, Listener).get_selected() {
                    let id = listener.id.clone();

                    if let Some(ref db) = app.db {
                        let args = ListenerIdParams { id: id.clone() };
                        // (found, the server's note)
                        let result = match open_state!(app, Listener).confirm_action {
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
                                open_state!(app, Listener)
                                    .set_success(format!("„{}”: {}", id, note));
                                open_state!(app, Listener).mode = ListenerModalMode::List;
                                open_state!(app, Listener).confirm_action = None;
                                refresh_listeners(app).await;
                            }
                            Ok((false, _)) => {
                                open_state!(app, Listener)
                                    .set_error(t().listener_not_found().to_string());
                                open_state!(app, Listener).cancel();
                                refresh_listeners(app).await;
                            }
                            Err(e) => {
                                open_state!(app, Listener).set_error(t().failed(Op::Update, &e));
                                open_state!(app, Listener).cancel();
                            }
                        }
                    }
                }
            }
            _ => {}
        },
    }
}
