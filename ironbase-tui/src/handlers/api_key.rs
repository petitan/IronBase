//! Key handlers: api_key

use crate::app::App;
use crate::db::DbWrapper;
use crate::modals;
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
        Ok(keys) => app.api_key_state.set_keys(keys),
        Err(e) => {
            let outcome = app
                .api_key_state
                .success
                .take()
                .or(app.api_key_state.error.take());
            let reload = format!("reloading the API key list failed: {}", e);
            app.api_key_state.set_error(match outcome {
                Some(outcome) => format!("{} ({})", outcome, reload),
                None => reload,
            });
        }
    }
}

/// Handle API Key modal keys
pub(crate) async fn handle_api_key_key_async(app: &mut App, key: KeyCode, modifiers: KeyModifiers) {
    use crate::modals::api_key::{ApiKeyModalMode, ConfirmAction};

    match app.api_key_state.mode {
        ApiKeyModalMode::List => match key {
            KeyCode::Esc => {
                app.close_modal();
            }
            KeyCode::Down | KeyCode::Char('j') => {
                app.api_key_state.select_next();
            }
            KeyCode::Up | KeyCode::Char('k') => {
                app.api_key_state.select_prev();
            }
            KeyCode::Char('n') if app.api_key_state.has_admin_key() => {
                app.api_key_state.start_create();
            }
            KeyCode::Char('r') if app.api_key_state.has_admin_key() => {
                app.api_key_state.start_revoke();
            }
            KeyCode::Char('d') if app.api_key_state.has_admin_key() => {
                app.api_key_state.start_delete();
            }
            KeyCode::Char('c') => {
                // Copy new key to clipboard (if available)
                if let Some(ref key) = app.api_key_state.new_key {
                    if let Ok(mut clipboard) = arboard::Clipboard::new() {
                        if clipboard.set_text(key.clone()).is_ok() {
                            app.api_key_state
                                .set_success("Key copied to clipboard!".to_string());
                        }
                    }
                }
            }
            _ => {}
        },
        ApiKeyModalMode::Create => match key {
            KeyCode::Esc => {
                app.api_key_state.cancel();
            }
            KeyCode::Enter => {
                let name = app.api_key_state.input.value().trim().to_string();
                if name.is_empty() {
                    app.api_key_state
                        .set_error("Name cannot be empty".to_string());
                    return;
                }

                if let Some(ref admin_key) = app.api_key_state.admin_key.clone() {
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
                                app.api_key_state.new_key = Some(key.to_string());
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
                                let msg = match saved_path {
                                    Some(path) => {
                                        format!("API key '{}' created! Saved to {}", name, path)
                                    }
                                    None => format!("API key '{}' created!", name),
                                };
                                app.api_key_state.set_success(msg);
                                app.api_key_state.mode = ApiKeyModalMode::List;
                                app.api_key_state.input.clear();

                                // Refresh the keys list
                                refresh_api_keys(app, admin_key).await;
                            }
                            Err(e) => {
                                app.api_key_state
                                    .set_error(format!("Failed to create: {}", e));
                            }
                        }
                    }
                }
            }
            _ => {
                app.api_key_state.handle_input(Input::Key(key, modifiers));
            }
        },
        ApiKeyModalMode::Confirm => match key {
            KeyCode::Esc | KeyCode::Char('n') => {
                app.api_key_state.cancel();
            }
            KeyCode::Char('y') => {
                if let Some(selected) = app.api_key_state.get_selected() {
                    let id = selected.id;
                    let name = selected.name.clone();

                    if let Some(ref admin_key) = app.api_key_state.admin_key.clone() {
                        if let Some(ref db) = app.db {
                            let args = AdminApiKeyIdParams {
                                id,
                                admin_key: Some(admin_key.clone()),
                            };
                            let result = match app.api_key_state.confirm_action {
                                Some(ConfirmAction::Revoke) => {
                                    db.call::<AdminApikeyRevoke>(&args).await
                                }
                                Some(ConfirmAction::Delete) => {
                                    db.call::<AdminApikeyDelete>(&args).await
                                }
                                None => {
                                    app.api_key_state.cancel();
                                    return;
                                }
                            }
                            .map(|response| matches!(response, ApiKeyChangeResponse::Done { .. }));

                            match result {
                                Ok(true) => {
                                    let action = match app.api_key_state.confirm_action {
                                        Some(ConfirmAction::Revoke) => "revoked",
                                        Some(ConfirmAction::Delete) => "deleted",
                                        None => "modified",
                                    };
                                    app.api_key_state
                                        .set_success(format!("Key '{}' {}!", name, action));
                                    app.api_key_state.mode = ApiKeyModalMode::List;
                                    app.api_key_state.confirm_action = None;

                                    // Refresh the keys list
                                    refresh_api_keys(app, admin_key).await;
                                }
                                Ok(false) => {
                                    app.api_key_state.set_error("Key not found".to_string());
                                    app.api_key_state.cancel();
                                }
                                Err(e) => {
                                    app.api_key_state.set_error(format!("Failed: {}", e));
                                    app.api_key_state.cancel();
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
