//! Key handlers: acl

use crate::app::App;
use crate::db::DbWrapper;
use crate::i18n::{t, Op};
use crate::modals;
use crate::state::open_state;
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
        Ok(acls) => open_state!(app, Acl).set_acls(acls),
        Err(e) => {
            let outcome = open_state!(app, Acl)
                .success
                .take()
                .or(open_state!(app, Acl).error.take());
            let reload = t().failed(Op::ReloadList, &e);
            open_state!(app, Acl).set_error(match outcome {
                Some(outcome) => format!("{} ({})", outcome, reload),
                None => reload,
            });
        }
    }
}

/// ACL modal key handler
pub(crate) async fn handle_acl_key_async(app: &mut App, key: KeyCode, modifiers: KeyModifiers) {
    use crate::modals::acl::{AclModalMode, EditField};

    match open_state!(app, Acl).mode {
        AclModalMode::List => match key {
            KeyCode::Esc => {
                app.close_modal();
            }
            KeyCode::Down | KeyCode::Char('j') => {
                open_state!(app, Acl).select_next();
            }
            KeyCode::Up | KeyCode::Char('k') => {
                open_state!(app, Acl).select_prev();
            }
            KeyCode::Enter => {
                open_state!(app, Acl).enter_collection();
            }
            KeyCode::Char('e') if open_state!(app, Acl).is_localhost => {
                open_state!(app, Acl).start_edit_existing();
            }
            KeyCode::Char('d') if open_state!(app, Acl).is_localhost => {
                open_state!(app, Acl).start_delete();
            }
            _ => {}
        },
        AclModalMode::ViewCollection => match key {
            KeyCode::Esc | KeyCode::Backspace => {
                open_state!(app, Acl).back_to_list();
            }
            KeyCode::Down | KeyCode::Char('j') => {
                open_state!(app, Acl).select_next();
            }
            KeyCode::Up | KeyCode::Char('k') => {
                open_state!(app, Acl).select_prev();
            }
            _ => {}
        },
        AclModalMode::ConfirmDelete => match key {
            KeyCode::Esc | KeyCode::Char('n') => {
                open_state!(app, Acl).cancel_delete();
            }
            KeyCode::Char('y') => {
                if let Some(acl) = open_state!(app, Acl).get_selected() {
                    let collection = acl.collection.clone();
                    if let Some(ref db) = app.db {
                        let args = AclCollectionParams {
                            collection: collection.clone(),
                        };
                        // `success` is always true: `deleted` says whether it existed
                        match db.call::<AclDelete>(&args).await.map(|r| r.deleted) {
                            Ok(true) => {
                                open_state!(app, Acl).set_success(t().acl_deleted(&collection));
                                open_state!(app, Acl).mode = AclModalMode::List;
                                refresh_acls(app).await;
                            }
                            Ok(false) => {
                                // Someone else deleted it: the row is stale
                                open_state!(app, Acl).set_error(t().acl_not_found().to_string());
                                open_state!(app, Acl).cancel_delete();
                                refresh_acls(app).await;
                            }
                            Err(e) => {
                                open_state!(app, Acl).set_error(t().failed(Op::Delete, &e));
                                open_state!(app, Acl).cancel_delete();
                            }
                        }
                    }
                }
            }
            _ => {}
        },
        AclModalMode::Edit => {
            if open_state!(app, Acl).is_adding_rule {
                // Adding a new rule
                match key {
                    KeyCode::Esc => {
                        open_state!(app, Acl).cancel_add_rule();
                    }
                    KeyCode::Tab => {
                        open_state!(app, Acl).next_field();
                    }
                    KeyCode::BackTab => {
                        open_state!(app, Acl).prev_field();
                    }
                    KeyCode::Enter => {
                        open_state!(app, Acl).confirm_add_rule();
                    }
                    KeyCode::Left
                        if open_state!(app, Acl).edit_field == EditField::PrincipalType =>
                    {
                        open_state!(app, Acl).cycle_principal_type(false);
                    }
                    KeyCode::Right
                        if open_state!(app, Acl).edit_field == EditField::PrincipalType =>
                    {
                        open_state!(app, Acl).cycle_principal_type(true);
                    }
                    KeyCode::Char(' ') => match open_state!(app, Acl).edit_field {
                        EditField::PermRead | EditField::PermWrite | EditField::PermAdmin => {
                            open_state!(app, Acl).toggle_permission();
                        }
                        EditField::PrincipalType => {
                            open_state!(app, Acl).cycle_principal_type(true);
                        }
                        _ => {}
                    },
                    _ if open_state!(app, Acl).edit_field == EditField::PrincipalValue => {
                        open_state!(app, Acl).handle_principal_input(Input::Key(key, modifiers));
                    }
                    _ => {}
                }
            } else {
                // Browsing rules list
                match key {
                    KeyCode::Esc => {
                        open_state!(app, Acl).cancel_edit();
                    }
                    KeyCode::Down | KeyCode::Char('j') => {
                        open_state!(app, Acl).select_next();
                    }
                    KeyCode::Up | KeyCode::Char('k') => {
                        open_state!(app, Acl).select_prev();
                    }
                    KeyCode::Char('n') => {
                        open_state!(app, Acl).start_add_rule();
                    }
                    KeyCode::Char('d') => {
                        open_state!(app, Acl).delete_selected_rule();
                    }
                    KeyCode::Enter => {
                        open_state!(app, Acl).start_save();
                    }
                    _ => {}
                }
            }
        }
        AclModalMode::ConfirmSave => match key {
            KeyCode::Esc | KeyCode::Char('n') => {
                open_state!(app, Acl).cancel_save();
            }
            KeyCode::Char('y') => {
                let collection = open_state!(app, Acl).edit_collection.clone();
                let rules = open_state!(app, Acl).get_rules_json();

                if let Some(ref db) = app.db {
                    let args = AclSetParams {
                        collection: collection.clone(),
                        rules,
                    };
                    match db.call::<AclSet>(&args).await {
                        Ok(response) => {
                            open_state!(app, Acl)
                                .set_success(t().acl_saved(&collection, response.rules_count));
                            open_state!(app, Acl).mode = AclModalMode::List;
                            open_state!(app, Acl).edit_rules.clear();
                            open_state!(app, Acl).edit_collection.clear();
                            refresh_acls(app).await;
                        }
                        Err(e) => {
                            open_state!(app, Acl).set_error(t().failed(Op::Save, &e));
                            open_state!(app, Acl).cancel_save();
                        }
                    }
                }
            }
            _ => {}
        },
    }
}
