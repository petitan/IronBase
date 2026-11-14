//! Key handlers: api_key

use crate::app::App;
use crate::modals;
use crossterm::event::KeyCode;

/// Handle API Key modal keys
pub(crate) async fn handle_api_key_key_async(app: &mut App, key: KeyCode) {
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
                let name = app.api_key_state.input.trim().to_string();
                if name.is_empty() {
                    app.api_key_state
                        .set_error("Name cannot be empty".to_string());
                    return;
                }

                if let Some(ref admin_key) = app.api_key_state.admin_key.clone() {
                    if let Some(ref db) = app.db {
                        match db.create_api_key(admin_key, &name).await {
                            Ok(result) => {
                                // Extract the full key from the result
                                let mut saved_path: Option<String> = None;
                                if let Some(key) = result.get("key").and_then(|v| v.as_str()) {
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
                                                    if let Ok(username) = std::env::var("USERNAME")
                                                    {
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
                                if let Ok(keys) = db.list_api_keys(admin_key).await {
                                    let infos: Vec<_> = keys
                                        .iter()
                                        .filter_map(modals::api_key::ApiKeyInfo::from_value)
                                        .collect();
                                    app.api_key_state.set_keys(infos);
                                }
                            }
                            Err(e) => {
                                app.api_key_state
                                    .set_error(format!("Failed to create: {}", e));
                            }
                        }
                    }
                }
            }
            KeyCode::Char(c) => {
                app.api_key_state.input.push(c);
            }
            KeyCode::Backspace => {
                app.api_key_state.input.pop();
            }
            _ => {}
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
                            let result = match app.api_key_state.confirm_action {
                                Some(ConfirmAction::Revoke) => {
                                    db.revoke_api_key(admin_key, id).await
                                }
                                Some(ConfirmAction::Delete) => {
                                    db.delete_api_key(admin_key, id).await
                                }
                                None => Ok(false),
                            };

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
                                    if let Ok(keys) = db.list_api_keys(admin_key).await {
                                        let infos: Vec<_> = keys
                                            .iter()
                                            .filter_map(modals::api_key::ApiKeyInfo::from_value)
                                            .collect();
                                        app.api_key_state.set_keys(infos);
                                    }
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
