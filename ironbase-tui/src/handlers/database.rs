//! Key handlers: database

use crate::app::{App, Modal};
use crate::config::TransportMode;
use crate::db::DbWrapper;
use crossterm::event::KeyCode;

/// Open database modal
pub(crate) fn handle_database_open(app: &mut App) {
    let is_http = app.config.transport == TransportMode::Http;
    let current_path = app.db_path.as_ref().map(|p| p.display().to_string());
    app.database_state = crate::app::DatabaseState::new(current_path.as_deref(), is_http);
    app.modal = Some(Modal::Database);
}

/// Handle database modal keys
pub(crate) async fn handle_database_key_async(app: &mut App, key: KeyCode) {
    use crate::app::DatabaseMode;

    match key {
        KeyCode::Esc => {
            app.close_modal();
        }
        KeyCode::Tab => {
            // Toggle between Open and Create mode
            app.database_state.toggle_mode();
        }
        KeyCode::Enter => {
            let path = app.database_state.path.trim().to_string();
            if path.is_empty() {
                app.database_state.error = Some("Az utvonal nem lehet ures!".to_string());
                return;
            }

            let create = app.database_state.mode == DatabaseMode::Create;

            // HTTP mode: use db_open MCP tool
            if app.database_state.is_http_mode {
                if let Some(ref db) = app.db {
                    app.database_state.loading = true;
                    app.database_state.error = None;

                    match db.db_open(&path, create).await {
                        Ok(()) => {
                            app.database_state.loading = false;
                            let action = if create { "letrehozva" } else { "megnyitva" };
                            app.database_state.message = Some(format!("Sikeresen {}!", action));

                            // Reset UI state and refresh collections
                            app.reset_for_new_database();
                            let refreshed = app.refresh_collections_async().await;

                            // Close modal after success
                            app.close_modal();
                            // set_status clears a stale error, so the load error must win
                            match refreshed {
                                Ok(()) => app.set_status(format!("Adatbazis {}: {}", action, path)),
                                Err(e) => app.set_error(format!("Kollekcio betoltes hiba: {}", e)),
                            }
                        }
                        Err(e) => {
                            app.database_state.loading = false;
                            app.database_state.error = Some(format!("{}", e));
                        }
                    }
                } else {
                    app.database_state.error = Some("Nincs aktiv kapcsolat!".to_string());
                }
                return;
            }

            // Stdio mode: restart MCP server with new database
            let path_buf = std::path::PathBuf::from(&path);
            let file_exists = path_buf.exists();

            // Validate based on mode
            match app.database_state.mode {
                DatabaseMode::Open => {
                    if !file_exists {
                        app.database_state.error =
                            Some("A fajl nem letezik! Hasznald a 'Letrehozas' modot.".to_string());
                        return;
                    }
                }
                DatabaseMode::Create => {
                    if file_exists {
                        app.database_state.error =
                            Some("A fajl mar letezik! Hasznald a 'Megnyitas' modot.".to_string());
                        return;
                    }
                }
            }

            app.database_state.loading = true;
            app.database_state.error = None;

            let server_path = app.config.get_mcp_server_path();

            // Close existing connection first
            if let Some(ref db) = app.db {
                let _ = db.close().await;
            }
            app.db = None;

            // Connect to database (MCP server will create if needed)
            match DbWrapper::connect_stdio(&server_path, &path_buf).await {
                Ok(db) => {
                    app.db = Some(db);
                    app.db_path = Some(path_buf);
                    app.database_state.loading = false;

                    let action = match app.database_state.mode {
                        DatabaseMode::Open => "megnyitva",
                        DatabaseMode::Create => "letrehozva",
                    };
                    app.database_state.message = Some(format!("Sikeresen {}!", action));

                    // Reset UI state and refresh collections
                    app.reset_for_new_database();
                    let refreshed = app.refresh_collections_async().await;

                    // Close modal after success
                    app.close_modal();
                    // set_status clears a stale error, so the load error must win
                    match refreshed {
                        Ok(()) => app.set_status(format!("Adatbazis {}: {}", action, path)),
                        Err(e) => app.set_error(format!("Kollekcio betoltes hiba: {}", e)),
                    }
                }
                Err(e) => {
                    app.database_state.loading = false;
                    app.database_state.error = Some(format!("Kapcsolodasi hiba: {}", e));
                }
            }
        }
        KeyCode::Char(c) => {
            app.database_state.insert_char(c);
        }
        KeyCode::Backspace => {
            app.database_state.backspace();
        }
        KeyCode::Delete => {
            app.database_state.delete();
        }
        KeyCode::Left => {
            app.database_state.move_left();
        }
        KeyCode::Right => {
            app.database_state.move_right();
        }
        KeyCode::Home => {
            app.database_state.home();
        }
        KeyCode::End => {
            app.database_state.end();
        }
        _ => {}
    }
}
