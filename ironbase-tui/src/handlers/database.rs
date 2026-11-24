//! Key handlers: database

use crate::app::{App, OpenModal};
use crate::config::TransportMode;
use crate::db::DbWrapper;
use crate::i18n::{t, Op};
use crate::state::open_state;
use crate::widgets::Input;
use crossterm::event::{KeyCode, KeyModifiers};
use ironbase_mcp_types::params::DbOpenParams;
use ironbase_mcp_types::tools::DbOpen;

/// Open database modal
pub(crate) fn handle_database_open(app: &mut App) {
    let is_http = app.config.transport == TransportMode::Http;
    let current_path = app.db_path.as_ref().map(|p| p.display().to_string());
    app.open_modal = Some(OpenModal::Database(crate::app::DatabaseState::new(
        current_path.as_deref(),
        is_http,
    )));
}

/// Handle database modal keys
pub(crate) async fn handle_database_key_async(
    app: &mut App,
    key: KeyCode,
    modifiers: KeyModifiers,
) {
    use crate::app::DatabaseMode;

    match key {
        KeyCode::Esc => {
            app.close_modal();
        }
        KeyCode::Tab => {
            // Toggle between Open and Create mode
            open_state!(app, Database).toggle_mode();
        }
        KeyCode::Enter => {
            let path = open_state!(app, Database).path.value().trim().to_string();
            if path.is_empty() {
                open_state!(app, Database).error = Some(t().path_required().to_string());
                return;
            }

            let create = open_state!(app, Database).mode == DatabaseMode::Create;

            // HTTP mode: use db_open MCP tool
            if open_state!(app, Database).is_http_mode {
                if let Some(ref db) = app.db {
                    open_state!(app, Database).loading = true;
                    open_state!(app, Database).error = None;

                    let args = DbOpenParams {
                        path: path.clone(),
                        create,
                    };
                    match db.call::<DbOpen>(&args).await {
                        Ok(_) => {
                            open_state!(app, Database).loading = false;
                            let ok = if create {
                                t().created_ok()
                            } else {
                                t().opened_ok()
                            };
                            open_state!(app, Database).message = Some(ok.to_string());

                            // Reset UI state and refresh collections
                            app.reset_for_new_database();
                            let refreshed = app.refresh_collections_async().await;

                            // Close modal after success
                            app.close_modal();
                            // set_status clears a stale error, so the load error must win
                            match refreshed {
                                Ok(()) if create => app.set_status(t().database_created(&path)),
                                Ok(()) => app.set_status(t().database_opened(&path)),
                                Err(e) => app.set_error(t().failed(Op::LoadCollections, &e)),
                            }
                        }
                        Err(e) => {
                            open_state!(app, Database).loading = false;
                            open_state!(app, Database).error = Some(format!("{}", e));
                        }
                    }
                } else {
                    open_state!(app, Database).error = Some(t().no_connection().to_string());
                }
                return;
            }

            // Stdio mode: restart MCP server with new database
            let path_buf = std::path::PathBuf::from(&path);
            let file_exists = path_buf.exists();

            // Validate based on mode
            match open_state!(app, Database).mode {
                DatabaseMode::Open => {
                    if !file_exists {
                        open_state!(app, Database).error =
                            Some(t().file_missing_use_create().to_string());
                        return;
                    }
                }
                DatabaseMode::Create => {
                    if file_exists {
                        open_state!(app, Database).error =
                            Some(t().file_exists_use_open().to_string());
                        return;
                    }
                }
            }

            open_state!(app, Database).loading = true;
            open_state!(app, Database).error = None;

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
                    open_state!(app, Database).loading = false;

                    let create = open_state!(app, Database).mode == DatabaseMode::Create;
                    let ok = if create {
                        t().created_ok()
                    } else {
                        t().opened_ok()
                    };
                    open_state!(app, Database).message = Some(ok.to_string());

                    // Reset UI state and refresh collections
                    app.reset_for_new_database();
                    let refreshed = app.refresh_collections_async().await;

                    // Close modal after success
                    app.close_modal();
                    // set_status clears a stale error, so the load error must win
                    match refreshed {
                        Ok(()) if create => app.set_status(t().database_created(&path)),
                        Ok(()) => app.set_status(t().database_opened(&path)),
                        Err(e) => app.set_error(t().failed(Op::LoadCollections, &e)),
                    }
                }
                Err(e) => {
                    open_state!(app, Database).loading = false;
                    open_state!(app, Database).error = Some(t().failed(Op::Connect, &e));
                }
            }
        }
        _ => {
            open_state!(app, Database).handle_input(Input::Key(key, modifiers));
        }
    }
}
