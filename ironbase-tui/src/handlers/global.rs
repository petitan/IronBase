//! Key handlers: global

use super::database::handle_database_open;
use super::fulltext::handle_fulltext_open;
use super::rag::handle_rag_open;
use super::server_info::handle_server_info_open;
use super::update::handle_update_open;
use super::vector::handle_vector_open;
use crate::app::{App, Modal};
use crate::i18n::{t, Op};
use crate::keymap::{self, Action};
use crate::modals;
use crossterm::event::{KeyCode, KeyModifiers};

/// Handle the main screen's keys: the key map (`crate::keymap::MAIN`) names
/// the action, `run_action` does it
pub(crate) async fn handle_global_key_async(app: &mut App, key: KeyCode, modifiers: KeyModifiers) {
    let ctx = keymap::Context {
        pane: app.active_pane,
        has_error: app.error_message.is_some(),
        has_filter: app.active_filter.is_some(),
        has_matches: !app.search.doc_matches.is_empty(),
    };
    if let Some(action) = keymap::resolve(key, modifiers, &ctx) {
        run_action(app, action).await;
    }
}

/// Do an action of the main screen or the actions menu
pub(crate) async fn run_action(app: &mut App, action: Action) {
    match action {
        Action::Quit => {
            app.should_quit = true;
        }

        // Pane navigation
        Action::NextPane => app.next_pane(),
        Action::PrevPane => app.prev_pane(),

        // List navigation - Arrow keys
        Action::Up => app.select_up_async().await,
        Action::Down => app.select_down_async().await,
        Action::PageUp => app.page_up_async().await,
        Action::PageDown => app.page_down_async().await,
        Action::GoToStart => app.go_to_start_async().await,
        Action::GoToEnd => app.go_to_end_async().await,

        // Modals
        Action::OpenSearch => app.open_search(),
        Action::OpenActions => app.open_actions(),
        Action::OpenHelp => app.open_help(),
        Action::ServerInfo => handle_server_info_open(app).await,
        Action::UpdateCheck => handle_update_open(app).await,
        Action::RefreshCollections => {
            // Refresh collections (Shift+R)
            app.set_status(t().refreshing());
            match app.refresh_collections_async().await {
                Ok(_) => app.set_status(t().refreshed()),
                Err(e) => app.set_error(t().failed(Op::Refresh, &e)),
            }
        }
        Action::Filter => app.open_filter_modal_async().await,
        Action::Fulltext => handle_fulltext_open(app).await,
        Action::VectorSearch => handle_vector_open(app).await,
        Action::Rag => handle_rag_open(app).await,

        // Error detail modal (if error present, 'e' opens error details)
        Action::ErrorDetails => {
            app.error_scroll = 0;
            app.set_modal(Modal::ErrorDetail);
        }

        // Edit (Detail pane, Documents pane) - only if no error message
        Action::EditDocument => {
            app.open_edit();
        }

        // Delete document (Detail pane, Documents pane)
        Action::DeleteDocument => {
            app.open_delete_confirm();
        }

        // Delete collection (Collections pane - Shift+D)
        Action::DeleteCollection => {
            app.open_delete_collection_confirm();
        }

        // Set ACL/Permission for collection (Collections pane - Shift+P)
        Action::EditAcl => {
            if !app.connection_type.is_localhost() {
                app.set_error(t().acl_needs_localhost());
            } else {
                // Clone collection name before borrowing app mutably
                let collection = app.current_collection_name().map(|s| s.to_string());
                if let Some(coll) = collection {
                    // Load the current ACL first: acl_set replaces the whole rule list,
                    // so an editor opened empty would overwrite the existing rules
                    let existing = match app.db {
                        Some(ref db) => db
                            .call::<ironbase_mcp_types::tools::AclGet>(
                                &ironbase_mcp_types::params::AclCollectionParams {
                                    collection: coll.clone(),
                                },
                            )
                            .await
                            .map_err(|e| e.to_string()),
                        None => Err(t().no_database().to_string()),
                    };
                    match existing {
                        Ok(acl) => {
                            let info = match acl {
                                ironbase_mcp_types::responses::AclGetResponse::Set(doc) => {
                                    Some(modals::acl::CollectionAclInfo::from_document(&doc))
                                }
                                ironbase_mcp_types::responses::AclGetResponse::NotSet {
                                    ..
                                } => None,
                            };
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
                                app.set_modal(Modal::Acl);
                            }
                        }
                        Err(e) => app.set_error(t().failed(Op::LoadAcl, &e)),
                    }
                }
            }
        }

        // Insert (Collections, Documents pane)
        Action::InsertDocument => {
            app.open_insert();
        }

        // Copy to clipboard (Detail or Documents pane)
        Action::CopyDocument => {
            app.copy_document_to_clipboard();
        }

        // Search navigation (Detail pane - n/N for next/prev match)
        Action::NextMatch => {
            app.goto_next_match();
        }
        Action::PrevMatch => {
            app.goto_prev_match();
        }

        // Theme
        Action::NextTheme => app.next_theme(),

        // API Key management (Shift+K)
        Action::ApiKeys => {
            app.api_key_state = modals::api_key::ApiKeyState::new();
            app.api_key_state.is_localhost = app.connection_type.is_localhost();
            app.set_modal(Modal::ApiKey);
            // Load API keys if admin key is available and can_admin
            if app.api_key_state.can_admin() {
                if let Some(ref admin_key) = app.api_key_state.admin_key.clone() {
                    if let Some(ref db) = app.db {
                        app.api_key_state.loading = true;
                        match super::api_key::load_api_keys(db, admin_key).await {
                            Ok(keys) => {
                                app.api_key_state.set_keys(keys);
                                app.api_key_state.loading = false;
                            }
                            Err(e) => {
                                app.api_key_state.set_error(t().failed(Op::LoadApiKeys, &e));
                                app.api_key_state.loading = false;
                            }
                        }
                    }
                }
            }
        }

        // ACL management (Shift+A)
        Action::AclRules => {
            app.acl_state = modals::acl::AclState::new();
            app.acl_state.is_localhost = app.connection_type.is_localhost();
            app.set_modal(Modal::Acl);
            // Load ACL rules
            if let Some(ref db) = app.db {
                app.acl_state.loading = true;
                match super::acl::load_acls(db).await {
                    Ok(acls) => {
                        app.acl_state.set_acls(acls);
                        app.acl_state.loading = false;
                    }
                    Err(e) => {
                        app.acl_state.set_error(t().failed(Op::LoadAcl, &e));
                        app.acl_state.loading = false;
                    }
                }
            }
        }

        // Listener management (Shift+L)
        Action::Listeners => {
            app.listener_state = modals::listener::ListenerState::new();
            app.listener_state.is_localhost = app.connection_type.is_localhost();
            app.set_modal(Modal::Listener);
            // Load listeners
            if let Some(ref db) = app.db {
                app.listener_state.loading = true;
                match super::listener::load_listeners(db).await {
                    Ok(listeners) => {
                        app.listener_state.set_listeners(listeners);
                        app.listener_state.loading = false;
                    }
                    Err(e) => {
                        app.listener_state
                            .set_error(t().failed(Op::LoadListeners, &e));
                        app.listener_state.loading = false;
                    }
                }
            }
        }

        // Refresh (r) - reload collections and documents
        // BUG FIX: refresh_collections_async already calls refresh_documents_inner,
        // so we don't need to call refresh_documents_async again
        Action::Refresh => {
            app.set_status(t().refreshing());
            match app.refresh_collections_async().await {
                Ok(_) => app.set_status(t().refreshed()),
                Err(e) => app.set_error(t().failed(Op::Refresh, &e)),
            }
        }

        // Clear filter (Escape in Documents pane when filter is active)
        Action::ClearFilter => {
            app.active_filter = None;
            app.active_sort = None;
            app.doc_scroll_offset = 0;
            app.selected_document = 0;
            match app.refresh_documents_async().await {
                Ok(_) => app.set_status(t().filter_cleared()),
                Err(e) => app.set_error(t().failed(Op::Refresh, &e)),
            }
        }

        // The actions menu's own actions
        Action::NewCollection => app.open_new_collection(),
        Action::Indexes => {
            app.open_index_modal();
            app.load_indexes_async().await;
            app.load_index_statistics_async().await;
        }
        Action::Query => app.open_query_modal(),
        Action::Export => app.open_export_modal(),
        Action::Scripts => {
            app.open_script_modal();
            app.load_scripts_async().await;
        }
        Action::OpenDatabase => handle_database_open(app),
    }
}
