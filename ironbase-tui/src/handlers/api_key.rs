//! Key handlers: api_key

use crate::app::App;
use crate::db::DbWrapper;
use crate::i18n::{t, Op};
use crate::modals;
use crate::state::open_state;
use crate::widgets::Input;
use crossterm::event::{KeyCode, KeyModifiers};
use ironbase_mcp_types::params::{AdminApiKeyCreateParams, AdminApiKeyIdParams, AdminKeyParams};
use ironbase_mcp_types::responses::ApiKeyChangeResponse;
use ironbase_mcp_types::tools::{
    AdminApikeyCreate, AdminApikeyDelete, AdminApikeyList, AdminApikeyRevoke,
};

/// The server's API keys, masked (`admin_apikey_list`)
pub(crate) async fn load_api_keys(
    db: &DbWrapper,
    admin_key: &str,
) -> anyhow::Result<Vec<modals::api_key::ApiKeyInfo>> {
    let args = AdminKeyParams {
        admin_key: Some(admin_key.to_string()),
    };
    Ok(db.call::<AdminApikeyList>(&args).await?.keys)
}

/// Reload the key list after a change; a failed reload is reported next to
/// the change's own outcome (which it does not hide)
async fn refresh_api_keys(app: &mut App, admin_key: &str) {
    let Some(ref db) = app.db else { return };
    match load_api_keys(db, admin_key).await {
        Ok(keys) => open_state!(app, ApiKey).set_keys(keys),
        Err(e) => {
            let outcome = open_state!(app, ApiKey)
                .success
                .take()
                .or(open_state!(app, ApiKey).error.take());
            let reload = t().failed(Op::ReloadList, &e);
            open_state!(app, ApiKey).set_error(match outcome {
                Some(outcome) => format!("{} ({})", outcome, reload),
                None => reload,
            });
        }
    }
}

/// Handle API Key modal keys
pub(crate) async fn handle_api_key_key_async(app: &mut App, key: KeyCode, modifiers: KeyModifiers) {
    use crate::modals::api_key::{ApiKeyModalMode, ConfirmAction};

    match open_state!(app, ApiKey).mode {
        ApiKeyModalMode::List => match key {
            KeyCode::Esc => {
                app.close_modal();
            }
            KeyCode::Down | KeyCode::Char('j') => {
                open_state!(app, ApiKey).select_next();
            }
            KeyCode::Up | KeyCode::Char('k') => {
                open_state!(app, ApiKey).select_prev();
            }
            KeyCode::Char('n') if open_state!(app, ApiKey).has_admin_key() => {
                open_state!(app, ApiKey).start_create();
            }
            KeyCode::Char('r') if open_state!(app, ApiKey).has_admin_key() => {
                open_state!(app, ApiKey).start_revoke();
            }
            KeyCode::Char('d') if open_state!(app, ApiKey).has_admin_key() => {
                open_state!(app, ApiKey).start_delete();
            }
            KeyCode::Char('c') => {
                // Copy new key to clipboard (if available)
                if let Some(ref key) = open_state!(app, ApiKey).new_key {
                    if let Ok(mut clipboard) = arboard::Clipboard::new() {
                        if clipboard.set_text(key.clone()).is_ok() {
                            open_state!(app, ApiKey).set_success(t().key_copied().to_string());
                        }
                    }
                }
            }
            _ => {}
        },
        ApiKeyModalMode::Create => match key {
            KeyCode::Esc => {
                open_state!(app, ApiKey).cancel();
            }
            KeyCode::Enter => {
                let name = open_state!(app, ApiKey).input.value().trim().to_string();
                if name.is_empty() {
                    open_state!(app, ApiKey).set_error(t().name_required().to_string());
                    return;
                }

                if let Some(ref admin_key) = open_state!(app, ApiKey).admin_key.clone() {
                    if let Some(ref db) = app.db {
                        let args = AdminApiKeyCreateParams {
                            name: name.clone(),
                            admin_key: Some(admin_key.clone()),
                        };
                        match db.call::<AdminApikeyCreate>(&args).await {
                            Ok(created) => {
                                let key = created.key.as_str();
                                let mut saved_path: Option<String> = None;
                                // The TUI holds the new key from here: masked in the log
                                crate::logging::register_secret(key);
                                open_state!(app, ApiKey).new_key = Some(key.to_string());
                                // Save key to config directory for easy copying
                                if let Some(config_dir) = dirs::config_dir() {
                                    let tui_dir = config_dir.join("ironbase-tui");
                                    let _ = std::fs::create_dir_all(&tui_dir);
                                    let key_file = tui_dir.join("new_key.txt");

                                    // Write file with restrictive permissions from the start
                                    let write_result = {
                                        #[cfg(unix)]
                                        {
                                            use std::fs::OpenOptions;
                                            use std::io::Write;
                                            use std::os::unix::fs::OpenOptionsExt;
                                            OpenOptions::new()
                                                .write(true)
                                                .create(true)
                                                .truncate(true)
                                                .mode(0o600)
                                                .open(&key_file)
                                                .and_then(|mut f| {
                                                    f.write_all(format!("{}\n", key).as_bytes())
                                                })
                                        }
                                        #[cfg(not(unix))]
                                        {
                                            std::fs::write(&key_file, format!("{}\n", key))
                                        }
                                    };

                                    if write_result.is_ok() {
                                        saved_path = Some(key_file.display().to_string());

                                        #[cfg(windows)]
                                        {
                                            // Use icacls to set owner-only permissions
                                            use std::os::windows::process::CommandExt;
                                            if let Some(path_str) = key_file.to_str() {
                                                if let Ok(username) = std::env::var("USERNAME") {
                                                    // Quote the path for spaces
                                                    let _ = std::process::Command::new("icacls")
                                                        .args([
                                                            &format!("\"{}\"", path_str),
                                                            "/inheritance:r",
                                                            "/grant:r",
                                                            &format!("\"{}\":F", username),
                                                        ])
                                                        .creation_flags(0x08000000) // CREATE_NO_WINDOW
                                                        .output();
                                                }
                                            }
                                        }
                                    }
                                }
                                let msg = t().api_key_created(&name, saved_path.as_deref());
                                open_state!(app, ApiKey).set_success(msg);
                                open_state!(app, ApiKey).mode = ApiKeyModalMode::List;
                                open_state!(app, ApiKey).input.clear();

                                // Refresh the keys list
                                refresh_api_keys(app, admin_key).await;
                            }
                            Err(e) => {
                                open_state!(app, ApiKey).set_error(t().failed(Op::Create, &e));
                            }
                        }
                    }
                }
            }
            _ => {
                open_state!(app, ApiKey).handle_input(Input::Key(key, modifiers));
            }
        },
        ApiKeyModalMode::Confirm => match key {
            KeyCode::Esc | KeyCode::Char('n') => {
                open_state!(app, ApiKey).cancel();
            }
            KeyCode::Char('y') => {
                if let Some(selected) = open_state!(app, ApiKey).get_selected() {
                    let id = selected.id;
                    let name = selected.name.clone();

                    if let Some(ref admin_key) = open_state!(app, ApiKey).admin_key.clone() {
                        if let Some(ref db) = app.db {
                            let args = AdminApiKeyIdParams {
                                id,
                                admin_key: Some(admin_key.clone()),
                            };
                            let result = match open_state!(app, ApiKey).confirm_action {
                                Some(ConfirmAction::Revoke) => {
                                    db.call::<AdminApikeyRevoke>(&args).await
                                }
                                Some(ConfirmAction::Delete) => {
                                    db.call::<AdminApikeyDelete>(&args).await
                                }
                                None => {
                                    open_state!(app, ApiKey).cancel();
                                    return;
                                }
                            }
                            .map(|response| matches!(response, ApiKeyChangeResponse::Done { .. }));

                            match result {
                                Ok(true) => {
                                    let message = match open_state!(app, ApiKey).confirm_action {
                                        Some(ConfirmAction::Delete) => t().api_key_deleted(&name),
                                        _ => t().api_key_revoked(&name),
                                    };
                                    open_state!(app, ApiKey).set_success(message);
                                    open_state!(app, ApiKey).mode = ApiKeyModalMode::List;
                                    open_state!(app, ApiKey).confirm_action = None;

                                    // Refresh the keys list
                                    refresh_api_keys(app, admin_key).await;
                                }
                                Ok(false) => {
                                    open_state!(app, ApiKey)
                                        .set_error(t().key_not_found().to_string());
                                    open_state!(app, ApiKey).cancel();
                                }
                                Err(e) => {
                                    open_state!(app, ApiKey).set_error(t().failed(Op::Update, &e));
                                    open_state!(app, ApiKey).cancel();
                                }
                            }
                        }
                    }
                }
            }
            _ => {}
        },
    }
}
