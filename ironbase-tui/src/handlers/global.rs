//! Key handlers: global

use super::fulltext::handle_fulltext_open;
use super::rag::handle_rag_open;
use super::server_info::handle_server_info_open;
use super::update::handle_update_open;
use super::vector::handle_vector_open;
use crate::app::{App, Modal, Pane};
use crate::modals;
use crossterm::event::{KeyCode, KeyModifiers};

/// Handle global keys with async support
pub(crate) async fn handle_global_key_async(app: &mut App, key: KeyCode, modifiers: KeyModifiers) {
    match (key, modifiers) {
        // Quit
        (KeyCode::Char('c'), KeyModifiers::CONTROL) | (KeyCode::Char('q'), _) => {
            app.should_quit = true;
        }

        // Pane navigation
        (KeyCode::Tab, KeyModifiers::NONE) => app.next_pane(),
        (KeyCode::BackTab, _) => app.prev_pane(),

        // List navigation - Arrow keys
        (KeyCode::Up, _) => app.select_up_async().await,
        (KeyCode::Down, _) => app.select_down_async().await,
        (KeyCode::PageUp, _) => app.page_up_async().await,
        (KeyCode::PageDown, _) => app.page_down_async().await,
        (KeyCode::Home, _) => app.go_to_start_async().await,
        (KeyCode::End, _) => app.go_to_end_async().await,

        // List navigation - Vim keys
        (KeyCode::Char('k'), _) => app.select_up_async().await,
        (KeyCode::Char('j'), _) => app.select_down_async().await,
        (KeyCode::Char('g'), _) => app.go_to_start_async().await,
        (KeyCode::Char('G'), _) => app.go_to_end_async().await,

        // Modals
        (KeyCode::Char('/'), _) => app.open_search(),
        (KeyCode::Char('a'), _) => app.open_actions(),
        (KeyCode::Char('?'), _) => app.open_help(),
        (KeyCode::Char('I'), _) => handle_server_info_open(app).await,
        (KeyCode::Char('U'), _) => handle_update_open(app).await,
        (KeyCode::Char('R'), _) => {
            // Refresh collections (Shift+R)
            app.set_status("Refreshing collections...");
            match app.refresh_collections_async().await {
                Ok(_) => app.set_status("Collections refreshed!"),
                Err(e) => app.set_error(format!("Refresh failed: {}", e)),
            }
        }
        (KeyCode::Char('f'), _) => app.open_filter_modal_async().await,
        (KeyCode::Char('F'), KeyModifiers::SHIFT) => handle_fulltext_open(app).await,
        (KeyCode::Char('V'), KeyModifiers::SHIFT) => handle_vector_open(app).await,
        (KeyCode::Char('E'), KeyModifiers::SHIFT) => handle_rag_open(app).await,

        // Error detail modal (if error present, 'e' opens error details)
        (KeyCode::Char('e'), _) if app.error_message.is_some() => {
            app.error_scroll = 0;
            app.modal = Some(Modal::ErrorDetail);
        }

        // Edit (Detail pane, Documents pane) - only if no error message
        (KeyCode::Char('e'), _)
            if app.active_pane == Pane::Detail || app.active_pane == Pane::Documents =>
        {
            app.open_edit();
        }

        // Delete document (Detail pane, Documents pane)
        (KeyCode::Char('d'), _)
            if app.active_pane == Pane::Detail || app.active_pane == Pane::Documents =>
        {
            app.open_delete_confirm();
        }

        // Delete collection (Collections pane - Shift+D)
        (KeyCode::Char('D'), KeyModifiers::SHIFT) if app.active_pane == Pane::Collections => {
            app.open_delete_collection_confirm();
        }

        // Set ACL/Permission for collection (Collections pane - Shift+P)
        (KeyCode::Char('P'), KeyModifiers::SHIFT) if app.active_pane == Pane::Collections => {
            if !app.connection_type.is_localhost() {
                app.set_error("ACL editing requires localhost connection");
            } else {
                // Clone collection name before borrowing app mutably
                let collection = app.current_collection_name().map(|s| s.to_string());
                if let Some(coll) = collection {
                    // Load the current ACL first: acl_set replaces the whole rule list,
                    // so an editor opened empty would overwrite the existing rules
                    let existing = match app.db {
                        Some(ref db) => db.acl_get(&coll).await.map_err(|e| e.to_string()),
                        None => Err("Nincs megnyitva adatbázis".to_string()),
                    };
                    match existing {
                        Ok(acl) => {
                            let info = acl
                                .as_ref()
                                .and_then(modals::acl::CollectionAclInfo::from_value);
                            // Never load a mangled copy: saving it would
                            // corrupt the stored rules
                            if let Some(message) =
                                info.as_ref().and_then(|info| info.uneditable_message())
                            {
                                app.set_error(message);
                            } else {
                                let rules = info.map(|info| info.rules).unwrap_or_default();
                                app.acl_state = modals::acl::AclState::new();
                                app.acl_state.is_localhost = true;
                                app.acl_state.start_edit(coll);
                                app.acl_state.load_edit_rules(&rules);
                                app.modal = Some(Modal::Acl);
                            }
                        }
                        Err(e) => app.set_error(format!("ACL betoltese sikertelen: {}", e)),
                    }
                }
            }
        }

        // Insert (Collections, Documents pane)
        (KeyCode::Char('i'), _)
            if app.active_pane == Pane::Collections || app.active_pane == Pane::Documents =>
        {
            app.open_insert();
        }

        // Copy to clipboard (Detail or Documents pane)
        (KeyCode::Char('y'), _)
            if app.active_pane == Pane::Detail || app.active_pane == Pane::Documents =>
        {
            app.copy_document_to_clipboard();
        }

        // Search navigation (Detail pane - n/N for next/prev match)
        (KeyCode::Char('n'), _)
            if app.active_pane == Pane::Detail && !app.search.doc_matches.is_empty() =>
        {
            app.goto_next_match();
        }
        (KeyCode::Char('N'), KeyModifiers::SHIFT)
            if app.active_pane == Pane::Detail && !app.search.doc_matches.is_empty() =>
        {
            app.goto_prev_match();
        }

        // Theme
        (KeyCode::Char('t'), _) => app.next_theme(),

        // API Key management (Shift+K)
        (KeyCode::Char('K'), KeyModifiers::SHIFT) => {
            app.api_key_state = modals::api_key::ApiKeyState::new();
            app.api_key_state.is_localhost = app.connection_type.is_localhost();
            app.modal = Some(Modal::ApiKey);
            // Load API keys if admin key is available and can_admin
            if app.api_key_state.can_admin() {
                if let Some(ref admin_key) = app.api_key_state.admin_key.clone() {
                    if let Some(ref db) = app.db {
                        app.api_key_state.loading = true;
                        match db.list_api_keys(admin_key).await {
                            Ok(keys) => {
                                let infos: Vec<_> = keys
                                    .iter()
                                    .filter_map(modals::api_key::ApiKeyInfo::from_value)
                                    .collect();
                                app.api_key_state.set_keys(infos);
                                app.api_key_state.loading = false;
                            }
                            Err(e) => {
                                app.api_key_state
                                    .set_error(format!("Failed to load keys: {}", e));
                                app.api_key_state.loading = false;
                            }
                        }
                    }
                }
            }
        }

        // ACL management (Shift+A)
        (KeyCode::Char('A'), KeyModifiers::SHIFT) => {
            app.acl_state = modals::acl::AclState::new();
            app.acl_state.is_localhost = app.connection_type.is_localhost();
            app.modal = Some(Modal::Acl);
            // Load ACL rules
            if let Some(ref db) = app.db {
                app.acl_state.loading = true;
                match db.acl_list().await {
                    Ok(rules) => {
                        let acls: Vec<_> = rules
                            .iter()
                            .filter_map(modals::acl::CollectionAclInfo::from_value)
                            .collect();
                        app.acl_state.set_acls(acls);
                        app.acl_state.loading = false;
                    }
                    Err(e) => {
                        app.acl_state
                            .set_error(format!("Failed to load ACL: {}", e));
                        app.acl_state.loading = false;
                    }
                }
            }
        }

        // Listener management (Shift+L)
        (KeyCode::Char('L'), KeyModifiers::SHIFT) => {
            app.listener_state = modals::listener::ListenerState::new();
            app.listener_state.is_localhost = app.connection_type.is_localhost();
            app.modal = Some(Modal::Listener);
            // Load listeners
            if let Some(ref db) = app.db {
                app.listener_state.loading = true;
                match db.listener_list().await {
                    Ok(listeners) => {
                        let infos: Vec<_> = listeners
                            .iter()
                            .filter_map(modals::listener::ListenerInfo::from_value)
                            .collect();
                        app.listener_state.set_listeners(infos);
                        app.listener_state.loading = false;
                    }
                    Err(e) => {
                        app.listener_state
                            .set_error(format!("Failed to load listeners: {}", e));
                        app.listener_state.loading = false;
                    }
                }
            }
        }

        // Refresh (r) - reload collections and documents
        // BUG FIX: refresh_collections_async already calls refresh_documents_inner,
        // so we don't need to call refresh_documents_async again
        (KeyCode::Char('r'), _) => {
            app.set_status("Frissites...");
            match app.refresh_collections_async().await {
                Ok(_) => app.set_status("Frissitve!"),
                Err(e) => app.set_error(format!("Frissites hiba: {}", e)),
            }
        }

        // Clear filter (Escape in Documents pane when filter is active)
        (KeyCode::Esc, _) if app.active_filter.is_some() => {
            app.active_filter = None;
            app.active_sort = None;
            app.doc_scroll_offset = 0;
            app.selected_document = 0;
            match app.refresh_documents_async().await {
                Ok(_) => app.set_status("Szuro torolve"),
                Err(e) => app.set_error(format!("Frissites hiba: {}", e)),
            }
        }

        _ => {}
    }
}
