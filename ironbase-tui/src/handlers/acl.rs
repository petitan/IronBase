//! Key handlers: acl

use crate::app::App;
use crate::db::DbWrapper;
use crate::i18n::{t, Op};
use crate::modals;
use crate::widgets::Input;
use crossterm::event::{KeyCode, KeyModifiers};
use ironbase_mcp_types::params::{AclCollectionParams, AclSetParams, NoParams};
use ironbase_mcp_types::tools::{AclDelete, AclList, AclSet};

/// The server's custom ACLs (`acl_list`)
pub(crate) async fn load_acls(
    db: &DbWrapper,
) -> anyhow::Result<Vec<modals::acl::CollectionAclInfo>> {
    let response = db.call::<AclList>(&NoParams {}).await?;
    Ok(response
        .rules
        .iter()
        .map(modals::acl::CollectionAclInfo::from_document)
        .collect())
}

/// Reload the ACL list after a change; a failed reload is reported next to
/// the change's own outcome (which it does not hide)
async fn refresh_acls(app: &mut App) {
    let Some(ref db) = app.db else { return };
    match load_acls(db).await {
        Ok(acls) => app.acl_state.set_acls(acls),
        Err(e) => {
            let outcome = app.acl_state.success.take().or(app.acl_state.error.take());
            let reload = t().failed(Op::ReloadList, &e);
            app.acl_state.set_error(match outcome {
                Some(outcome) => format!("{} ({})", outcome, reload),
                None => reload,
            });
        }
    }
}

/// ACL modal key handler
pub(crate) async fn handle_acl_key_async(app: &mut App, key: KeyCode, modifiers: KeyModifiers) {
    use crate::modals::acl::{AclModalMode, EditField};

    match app.acl_state.mode {
        AclModalMode::List => match key {
            KeyCode::Esc => {
                app.close_modal();
            }
            KeyCode::Down | KeyCode::Char('j') => {
                app.acl_state.select_next();
            }
            KeyCode::Up | KeyCode::Char('k') => {
                app.acl_state.select_prev();
            }
            KeyCode::Enter => {
                app.acl_state.enter_collection();
            }
            KeyCode::Char('e') if app.acl_state.is_localhost => {
                app.acl_state.start_edit_existing();
            }
            KeyCode::Char('d') if app.acl_state.is_localhost => {
                app.acl_state.start_delete();
            }
            _ => {}
        },
        AclModalMode::ViewCollection => match key {
            KeyCode::Esc | KeyCode::Backspace => {
                app.acl_state.back_to_list();
            }
            KeyCode::Down | KeyCode::Char('j') => {
                app.acl_state.select_next();
            }
            KeyCode::Up | KeyCode::Char('k') => {
                app.acl_state.select_prev();
            }
            _ => {}
        },
        AclModalMode::ConfirmDelete => match key {
            KeyCode::Esc | KeyCode::Char('n') => {
                app.acl_state.cancel_delete();
            }
            KeyCode::Char('y') => {
                if let Some(acl) = app.acl_state.get_selected() {
                    let collection = acl.collection.clone();
                    if let Some(ref db) = app.db {
                        let args = AclCollectionParams {
                            collection: collection.clone(),
                        };
                        // `success` is always true: `deleted` says whether it existed
                        match db.call::<AclDelete>(&args).await.map(|r| r.deleted) {
                            Ok(true) => {
                                app.acl_state.set_success(t().acl_deleted(&collection));
                                app.acl_state.mode = AclModalMode::List;
                                refresh_acls(app).await;
                            }
                            Ok(false) => {
                                // Someone else deleted it: the row is stale
                                app.acl_state.set_error(t().acl_not_found().to_string());
                                app.acl_state.cancel_delete();
                                refresh_acls(app).await;
                            }
                            Err(e) => {
                                app.acl_state.set_error(t().failed(Op::Delete, &e));
                                app.acl_state.cancel_delete();
                            }
                        }
                    }
                }
            }
            _ => {}
        },
        AclModalMode::Edit => {
            if app.acl_state.is_adding_rule {
                // Adding a new rule
                match key {
                    KeyCode::Esc => {
                        app.acl_state.cancel_add_rule();
                    }
                    KeyCode::Tab => {
                        app.acl_state.next_field();
                    }
                    KeyCode::BackTab => {
                        app.acl_state.prev_field();
                    }
                    KeyCode::Enter => {
                        app.acl_state.confirm_add_rule();
                    }
                    KeyCode::Left if app.acl_state.edit_field == EditField::PrincipalType => {
                        app.acl_state.cycle_principal_type(false);
                    }
                    KeyCode::Right if app.acl_state.edit_field == EditField::PrincipalType => {
                        app.acl_state.cycle_principal_type(true);
                    }
                    KeyCode::Char(' ') => match app.acl_state.edit_field {
                        EditField::PermRead | EditField::PermWrite | EditField::PermAdmin => {
                            app.acl_state.toggle_permission();
                        }
                        EditField::PrincipalType => {
                            app.acl_state.cycle_principal_type(true);
                        }
                        _ => {}
                    },
                    _ if app.acl_state.edit_field == EditField::PrincipalValue => {
                        app.acl_state
                            .handle_principal_input(Input::Key(key, modifiers));
                    }
                    _ => {}
                }
            } else {
                // Browsing rules list
                match key {
                    KeyCode::Esc => {
                        app.acl_state.cancel_edit();
                    }
                    KeyCode::Down | KeyCode::Char('j') => {
                        app.acl_state.select_next();
                    }
                    KeyCode::Up | KeyCode::Char('k') => {
                        app.acl_state.select_prev();
                    }
                    KeyCode::Char('n') => {
                        app.acl_state.start_add_rule();
                    }
                    KeyCode::Char('d') => {
                        app.acl_state.delete_selected_rule();
                    }
                    KeyCode::Enter => {
                        app.acl_state.start_save();
                    }
                    _ => {}
                }
            }
        }
        AclModalMode::ConfirmSave => match key {
            KeyCode::Esc | KeyCode::Char('n') => {
                app.acl_state.cancel_save();
            }
            KeyCode::Char('y') => {
                let collection = app.acl_state.edit_collection.clone();
                let rules = app.acl_state.get_rules_json();

                if let Some(ref db) = app.db {
                    let args = AclSetParams {
                        collection: collection.clone(),
                        rules,
                    };
                    match db.call::<AclSet>(&args).await {
                        Ok(response) => {
                            app.acl_state
                                .set_success(t().acl_saved(&collection, response.rules_count));
                            app.acl_state.mode = AclModalMode::List;
                            app.acl_state.edit_rules.clear();
                            app.acl_state.edit_collection.clear();
                            refresh_acls(app).await;
                        }
                        Err(e) => {
                            app.acl_state.set_error(t().failed(Op::Save, &e));
                            app.acl_state.cancel_save();
                        }
                    }
                }
            }
            _ => {}
        },
    }
}
