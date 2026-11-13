//! Key handlers: acl

use crate::app::App;
use crate::modals;
use crossterm::event::KeyCode;

/// ACL modal key handler
pub(crate) async fn handle_acl_key_async(app: &mut App, key: KeyCode) {
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
                        match db.acl_delete(&collection).await {
                            Ok(true) => {
                                app.acl_state
                                    .set_success(format!("ACL for '{}' deleted!", collection));
                                app.acl_state.mode = AclModalMode::List;

                                // Refresh ACL list
                                if let Ok(acls) = db.acl_list().await {
                                    let infos: Vec<_> = acls
                                        .iter()
                                        .filter_map(modals::acl::CollectionAclInfo::from_value)
                                        .collect();
                                    app.acl_state.set_acls(infos);
                                }
                            }
                            Ok(false) => {
                                app.acl_state.set_error("ACL not found".to_string());
                                app.acl_state.cancel_delete();
                            }
                            Err(e) => {
                                app.acl_state.set_error(format!("Failed: {}", e));
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
                    KeyCode::Char(c) if app.acl_state.edit_field == EditField::PrincipalValue => {
                        app.acl_state.current_rule.principal_value.push(c);
                    }
                    KeyCode::Backspace if app.acl_state.edit_field == EditField::PrincipalValue => {
                        app.acl_state.current_rule.principal_value.pop();
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
                    match db.acl_set(&collection, &rules).await {
                        Ok(true) => {
                            app.acl_state
                                .set_success(format!("ACL for '{}' saved!", collection));
                            app.acl_state.mode = AclModalMode::List;
                            app.acl_state.edit_rules.clear();
                            app.acl_state.edit_collection.clear();

                            // Refresh ACL list
                            if let Ok(acls) = db.acl_list().await {
                                let infos: Vec<_> = acls
                                    .iter()
                                    .filter_map(modals::acl::CollectionAclInfo::from_value)
                                    .collect();
                                app.acl_state.set_acls(infos);
                            }
                        }
                        Ok(false) => {
                            app.acl_state.set_error("Failed to save ACL".to_string());
                            app.acl_state.cancel_save();
                        }
                        Err(e) => {
                            app.acl_state.set_error(format!("Failed: {}", e));
                            app.acl_state.cancel_save();
                        }
                    }
                }
            }
            _ => {}
        },
    }
}
