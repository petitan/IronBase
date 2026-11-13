//! Behaviour pins: real key events through `handle_key` against the fake
//! MCP server, asserting on the call log and the App state / rendered screen

use crate::app::{App, Modal, Pane};
use crate::config::Config;
use crate::modals::confirm::ConfirmOption;
use crate::state::rag::RagTab;
use crate::state::vector::VectorTab;
use crate::state::{ConnectionType, ScriptConfirmAction, ScriptMode};
use crate::testing::{
    app_with_server, numbered_docs, press, press_with, render_to_string, type_text, FakeServer,
};
use crossterm::event::{KeyCode, KeyModifiers};
use serde_json::json;
use std::sync::Arc;

/// "users" (30 docs) and "visits" (5 docs)
fn two_collections() -> Arc<FakeServer> {
    FakeServer::new(vec![
        ("users", numbered_docs(30)),
        ("visits", numbered_docs(5)),
    ])
}

async fn shift(app: &mut App, c: char) {
    press_with(app, KeyCode::Char(c), KeyModifiers::SHIFT).await;
}

// === Global navigation ===

#[tokio::test]
async fn test_tab_cycles_panes() {
    let server = two_collections();
    let mut app = app_with_server(&server).await;
    assert_eq!(app.active_pane, Pane::Collections);

    press(&mut app, KeyCode::Tab).await;
    assert_eq!(app.active_pane, Pane::Documents);
    press(&mut app, KeyCode::Tab).await;
    assert_eq!(app.active_pane, Pane::Detail);
    press(&mut app, KeyCode::Tab).await;
    assert_eq!(app.active_pane, Pane::Collections);

    press_with(&mut app, KeyCode::BackTab, KeyModifiers::SHIFT).await;
    assert_eq!(app.active_pane, Pane::Detail);
}

#[tokio::test]
async fn test_selection_keys_in_documents_pane() {
    let server = two_collections();
    let mut app = app_with_server(&server).await;
    press(&mut app, KeyCode::Tab).await;
    server.clear_calls();

    press(&mut app, KeyCode::Char('j')).await;
    assert_eq!(app.selected_document, 1);
    press(&mut app, KeyCode::Down).await;
    assert_eq!(app.selected_document, 2);
    press(&mut app, KeyCode::Char('k')).await;
    assert_eq!(app.selected_document, 1);
    press(&mut app, KeyCode::Up).await;
    assert_eq!(app.selected_document, 0);
    // At the top: stays
    press(&mut app, KeyCode::Up).await;
    assert_eq!(app.selected_document, 0);
    // Moving inside the loaded page needs no server call
    assert!(server.calls().is_empty());
}

#[tokio::test]
async fn test_page_down_and_end_in_documents_pane() {
    let server = two_collections();
    let mut app = app_with_server(&server).await;
    assert_eq!((app.page_size, app.total_docs), (20, 30));
    press(&mut app, KeyCode::Tab).await;
    server.clear_calls();

    // PageDown: the window moves by a page but never past the last full page
    press(&mut app, KeyCode::PageDown).await;
    assert_eq!((app.doc_scroll_offset, app.selected_document), (10, 20));
    let finds = server.calls_to("find");
    assert_eq!(finds.len(), 1);
    assert_eq!(finds[0]["collection"], "users");
    assert_eq!(finds[0]["skip"], 10);
    assert_eq!(finds[0]["limit"], 20);

    // End: last document selected, on the last row of the window
    press(&mut app, KeyCode::End).await;
    assert_eq!((app.doc_scroll_offset, app.selected_document), (10, 29));
    assert_eq!(
        app.get_selected_document().map(|d| d["_id"].clone()),
        Some(json!(29))
    );

    // Home / g: back to the first document
    press(&mut app, KeyCode::Home).await;
    assert_eq!((app.doc_scroll_offset, app.selected_document), (0, 0));
    press_with(&mut app, KeyCode::Char('G'), KeyModifiers::SHIFT).await;
    assert_eq!(app.selected_document, 29);
    press(&mut app, KeyCode::Char('g')).await;
    assert_eq!((app.doc_scroll_offset, app.selected_document), (0, 0));
}

/// Quit is signalled by `App::should_quit`; the event loop returns on it
#[tokio::test]
async fn test_q_and_ctrl_c_quit() {
    let server = two_collections();
    let mut app = app_with_server(&server).await;
    assert!(!app.should_quit);
    press(&mut app, KeyCode::Char('q')).await;
    assert!(app.should_quit);

    let mut app = app_with_server(&server).await;
    press_with(&mut app, KeyCode::Char('c'), KeyModifiers::CONTROL).await;
    assert!(app.should_quit);
}

// === Collection switch ===

#[tokio::test]
async fn test_collection_switch_loads_documents_and_clears_filter_and_sort() {
    let server = two_collections();
    let mut app = app_with_server(&server).await;
    app.active_filter = Some(json!({"even": true}));
    app.active_sort = Some(json!({"n": -1}));
    server.clear_calls();

    press(&mut app, KeyCode::Char('j')).await;

    assert_eq!(app.current_collection_name(), Some("visits"));
    assert_eq!(app.active_filter, None);
    assert_eq!(app.active_sort, None);
    assert_eq!(app.documents.len(), 5);
    assert_eq!(app.total_docs, 5);
    let finds = server.calls_to("find");
    assert_eq!(finds.len(), 1);
    assert_eq!(finds[0]["collection"], "visits");
    assert_eq!(finds[0]["query"], json!({}));
    assert!(finds[0].get("sort").is_none());
    assert!(render_to_string(&app, 120, 36).contains("visits"));

    // Up / k switch back
    press(&mut app, KeyCode::Char('k')).await;
    assert_eq!(app.current_collection_name(), Some("users"));
    assert_eq!(app.documents.len(), 20);
}

#[tokio::test]
async fn test_failed_document_refresh_after_switch_leaves_empty_list_and_error() {
    let server = two_collections();
    let mut app = app_with_server(&server).await;
    *server.failing_find.lock().unwrap() = Some("visits".to_string());

    press(&mut app, KeyCode::Down).await;

    assert_eq!(app.current_collection_name(), Some("visits"));
    assert!(app.documents.is_empty());
    assert_eq!(app.total_docs, 0);
    let error = app.error_message.clone().unwrap_or_default();
    assert!(
        error.contains("Dokumentumok betoltese sikertelen"),
        "{}",
        error
    );
    assert!(error.contains("find failed"), "{}", error);
}

// === Delete document ===

#[tokio::test]
async fn test_delete_document_needs_explicit_yes() {
    let server = two_collections();
    server.respond("delete_one", json!({"deleted_count": 1}));
    let mut app = app_with_server(&server).await;
    press(&mut app, KeyCode::Tab).await;
    press(&mut app, KeyCode::Char('j')).await;
    server.clear_calls();

    // `d` opens the confirm, Cancel preselected
    press(&mut app, KeyCode::Char('d')).await;
    assert_eq!(app.modal, Some(Modal::Confirm));
    assert_eq!(app.confirm.selected, ConfirmOption::Cancel);
    let screen = render_to_string(&app, 120, 36);
    assert!(screen.contains("Dokumentum torlese"), "{}", screen);

    // Enter on the default = cancel: nothing is deleted
    press(&mut app, KeyCode::Enter).await;
    assert_eq!(app.modal, None);
    assert!(server.calls().is_empty());

    // `y` deletes exactly the selected document
    press(&mut app, KeyCode::Char('d')).await;
    press(&mut app, KeyCode::Char('y')).await;
    assert_eq!(app.modal, None);
    assert_eq!(
        server.calls_to("delete_one"),
        vec![json!({"collection": "users", "filter": {"_id": 1}})]
    );
    assert_eq!(app.status_message.as_deref(), Some("Dokumentum törölve"));
}

// === Index list ===

#[tokio::test]
async fn test_index_list_drop_flow() {
    let server = two_collections();
    server.respond(
        "index_list",
        json!({"btree_indexes": ["users_id", "users_age"]}),
    );
    server.respond("index_stats", json!({"indexes": []}));
    server.respond("index_drop", json!({"success": true}));
    let mut app = app_with_server(&server).await;

    // Actions -> x opens the index modal and loads the list
    press(&mut app, KeyCode::Char('a')).await;
    assert_eq!(app.modal, Some(Modal::Actions));
    press(&mut app, KeyCode::Char('x')).await;
    assert_eq!(app.modal, Some(Modal::Index));
    assert_eq!(app.index_state.indexes, vec!["users_id", "users_age"]);
    server.clear_calls();

    // The primary `<collection>_id` index: no confirm, no call
    press(&mut app, KeyCode::Char('d')).await;
    assert_eq!(app.modal, Some(Modal::Index));
    assert!(app.index_state.error.is_some());
    assert!(server.calls().is_empty());

    // Another index: confirm; Esc and Enter (default Cancel) return to the index modal
    press(&mut app, KeyCode::Char('j')).await;
    press(&mut app, KeyCode::Char('d')).await;
    assert_eq!(app.modal, Some(Modal::Confirm));
    assert_eq!(app.confirm.selected, ConfirmOption::Cancel);
    press(&mut app, KeyCode::Esc).await;
    assert_eq!(app.modal, Some(Modal::Index));
    press(&mut app, KeyCode::Char('d')).await;
    press(&mut app, KeyCode::Enter).await;
    assert_eq!(app.modal, Some(Modal::Index));
    assert!(server.calls().is_empty());

    // `y`: exactly one index_drop
    press(&mut app, KeyCode::Char('d')).await;
    press(&mut app, KeyCode::Char('y')).await;
    assert_eq!(app.modal, Some(Modal::Index));
    assert_eq!(
        server.calls_to("index_drop"),
        vec![json!({"collection": "users", "index_name": "users_age"})]
    );
}

// === Confirmed actions: the call uses the names captured when the confirm opened ===

#[tokio::test]
async fn test_vector_index_drop_executes_with_captured_names() {
    let server = two_collections();
    server.respond(
        "index_list",
        json!({"btree_indexes": [], "indexes": [{"name": "users_emb_vec", "field": "emb", "dimension": 3}]}),
    );
    server.respond("index_drop", json!({"success": true}));
    let mut app = app_with_server(&server).await;

    shift(&mut app, 'V').await;
    assert_eq!(app.modal, Some(Modal::VectorSearch));
    press(&mut app, KeyCode::Tab).await;
    press(&mut app, KeyCode::Tab).await;
    assert_eq!(app.vector_state.active_tab, VectorTab::ListIndexes);
    server.clear_calls();

    press(&mut app, KeyCode::Char('d')).await;
    assert_eq!(app.modal, Some(Modal::Confirm));
    assert_eq!(app.confirm.selected, ConfirmOption::Cancel);
    // The modal state changing under the dialog must not change the target
    app.vector_state.collection = "other".to_string();
    app.vector_state.indexes.clear();

    press(&mut app, KeyCode::Char('y')).await;
    assert_eq!(app.modal, Some(Modal::VectorSearch));
    assert_eq!(
        server.calls_to("index_drop"),
        vec![json!({"collection": "users", "index_name": "users_emb_vec"})]
    );
}

#[tokio::test]
async fn test_script_rollback_executes_with_captured_names() {
    let server = two_collections();
    server.respond(
        "script_list",
        json!({"scripts": [{"name": "cleanup", "version": 3}]}),
    );
    server.respond(
        "script_get",
        json!({"name": "cleanup", "code": "1 + 1", "version": 3}),
    );
    server.respond(
        "script_history",
        json!({"history": [{"version": 3, "code": "a"}, {"version": 2, "code": "b"}]}),
    );
    server.respond(
        "script_rollback",
        json!({"success": true, "new_version": 4}),
    );
    let mut app = app_with_server(&server).await;

    press(&mut app, KeyCode::Char('a')).await;
    press(&mut app, KeyCode::Char('s')).await;
    assert_eq!(app.modal, Some(Modal::Script));
    assert_eq!(app.script_state.scripts.len(), 1);
    press(&mut app, KeyCode::Char('h')).await;
    assert_eq!(app.script_state.mode, ScriptMode::History);
    press(&mut app, KeyCode::Char('j')).await;
    assert_eq!(app.script_state.selected_version, 1);
    server.clear_calls();

    press(&mut app, KeyCode::Char('r')).await;
    assert_eq!(app.modal, Some(Modal::Confirm));
    assert_eq!(app.confirm.selected, ConfirmOption::Cancel);
    app.script_state.name = "other".to_string();
    app.script_state.selected_version = 0;

    press(&mut app, KeyCode::Char('y')).await;
    assert_eq!(app.modal, Some(Modal::Script));
    assert_eq!(
        server.calls_to("script_rollback"),
        vec![json!({"name": "cleanup", "version": 2})]
    );
    // The script is reloaded into the editor afterwards. (The success
    // message set by the rollback does not survive that reload today - not
    // pinned, see the phase report.)
    assert_eq!(
        server.calls_to("script_get").last(),
        Some(&json!({"name": "cleanup"}))
    );
    assert_eq!(app.script_state.mode, ScriptMode::Edit);
}

/// Shift+E, then BackTab to the Models tab (Tab is text input on Search)
async fn open_rag_models_tab(app: &mut App) {
    shift(app, 'E').await;
    assert_eq!(app.modal, Some(Modal::Rag));
    press_with(app, KeyCode::BackTab, KeyModifiers::SHIFT).await;
    press_with(app, KeyCode::BackTab, KeyModifiers::SHIFT).await;
    assert_eq!(app.rag_state.active_tab, RagTab::Models);
}

#[tokio::test]
async fn test_auto_embed_disable_executes_with_captured_collection() {
    let server = two_collections();
    server.respond("auto_embed_disable", json!({"success": true}));
    let mut app = app_with_server(&server).await;
    open_rag_models_tab(&mut app).await;
    server.clear_calls();

    press(&mut app, KeyCode::Char('d')).await;
    assert_eq!(app.modal, Some(Modal::Confirm));
    assert_eq!(app.confirm.selected, ConfirmOption::Cancel);
    app.rag_state.collection = "other".to_string();

    press(&mut app, KeyCode::Char('y')).await;
    assert_eq!(app.modal, Some(Modal::Rag));
    assert_eq!(
        server.calls_to("auto_embed_disable"),
        vec![json!({"collection": "users"})]
    );
    assert_eq!(
        app.rag_state.message.as_deref(),
        Some("Auto-embed disabled")
    );
}

#[tokio::test]
async fn test_embed_cache_clear_executes_after_confirm() {
    let server = two_collections();
    server.respond("embed_cache_clear", json!({"success": true}));
    let mut app = app_with_server(&server).await;
    open_rag_models_tab(&mut app).await;
    server.clear_calls();

    press(&mut app, KeyCode::Char('x')).await;
    assert_eq!(app.modal, Some(Modal::Confirm));
    // Enter on the default cancels
    press(&mut app, KeyCode::Enter).await;
    assert_eq!(app.modal, Some(Modal::Rag));
    assert!(server.calls().is_empty());

    press(&mut app, KeyCode::Char('x')).await;
    press(&mut app, KeyCode::Char('y')).await;
    assert_eq!(app.modal, Some(Modal::Rag));
    assert_eq!(server.calls_to("embed_cache_clear").len(), 1);
    assert_eq!(app.rag_state.message.as_deref(), Some("Cache cleared"));
}

#[tokio::test]
async fn test_embed_job_cancel_executes_with_captured_job_id() {
    let server = two_collections();
    server.respond(
        "embed_job_list",
        json!({"jobs": [{"job_id": "job-1"}, {"job_id": "job-2"}]}),
    );
    server.respond("embed_job_cancel", json!({"success": true}));
    let mut app = app_with_server(&server).await;
    shift(&mut app, 'E').await;
    press_with(&mut app, KeyCode::BackTab, KeyModifiers::SHIFT).await;
    assert_eq!(app.rag_state.active_tab, RagTab::Jobs);
    press(&mut app, KeyCode::Char('r')).await;
    assert_eq!(app.rag_state.jobs.len(), 2);
    press(&mut app, KeyCode::Char('j')).await;
    server.clear_calls();

    press(&mut app, KeyCode::Char('x')).await;
    assert_eq!(app.modal, Some(Modal::Confirm));
    assert_eq!(app.confirm.selected, ConfirmOption::Cancel);
    app.rag_state.selected_job = 0;

    press(&mut app, KeyCode::Char('y')).await;
    assert_eq!(app.modal, Some(Modal::Rag));
    assert_eq!(
        server.calls_to("embed_job_cancel"),
        vec![json!({"job_id": "job-2"})]
    );
    assert_eq!(
        app.rag_state.message.as_deref(),
        Some("Job job-2 cancelled")
    );
}

// === Script modal's own confirm ===

#[tokio::test]
async fn test_script_delete_confirm_enter_does_not_confirm() {
    let server = two_collections();
    server.respond(
        "script_list",
        json!({"scripts": [{"name": "cleanup", "version": 3}]}),
    );
    server.respond("script_delete", json!({"success": true}));
    let mut app = app_with_server(&server).await;
    press(&mut app, KeyCode::Char('a')).await;
    press(&mut app, KeyCode::Char('s')).await;
    server.clear_calls();

    press(&mut app, KeyCode::Char('d')).await;
    assert_eq!(
        app.script_state.confirm_action,
        Some(ScriptConfirmAction::DeleteScript)
    );
    press(&mut app, KeyCode::Enter).await;
    assert_eq!(app.script_state.confirm_action, None);
    assert_eq!(app.modal, Some(Modal::Script));
    assert!(server.calls().is_empty());

    // Only `y` deletes
    press(&mut app, KeyCode::Char('d')).await;
    press(&mut app, KeyCode::Char('y')).await;
    assert_eq!(
        server.calls_to("script_delete"),
        vec![json!({"name": "cleanup"})]
    );
}

#[tokio::test]
async fn test_script_discard_confirm_enter_does_not_confirm() {
    let server = two_collections();
    server.respond("script_list", json!({"scripts": []}));
    let mut app = app_with_server(&server).await;
    press(&mut app, KeyCode::Char('a')).await;
    press(&mut app, KeyCode::Char('s')).await;
    press(&mut app, KeyCode::Char('n')).await;
    assert_eq!(app.script_state.mode, ScriptMode::New);
    type_text(&mut app, "x").await;
    assert_eq!(app.script_state.name, "x");
    assert!(app.script_state.dirty);

    press(&mut app, KeyCode::Esc).await;
    assert_eq!(
        app.script_state.confirm_action,
        Some(ScriptConfirmAction::DiscardChanges)
    );
    press(&mut app, KeyCode::Enter).await;
    assert_eq!(app.script_state.confirm_action, None);
    assert_eq!(app.script_state.mode, ScriptMode::New);
    assert!(app.script_state.dirty);
    assert_eq!(app.script_state.name, "x");

    // `y` discards
    press(&mut app, KeyCode::Esc).await;
    press(&mut app, KeyCode::Char('y')).await;
    assert_eq!(app.script_state.mode, ScriptMode::Browse);
    assert!(!app.script_state.dirty);
}

// === Shift+P: collection ACL editor ===

#[tokio::test]
async fn test_shift_p_loads_stored_rules_into_editor() {
    let server = two_collections();
    server.respond(
        "acl_get",
        json!({
            "collection": "users",
            "rules": [{
                "principal": {"type": "apikey", "value": "abc"},
                "permissions": {"read": true, "write": false, "admin": false}
            }]
        }),
    );
    let mut app = app_with_server(&server).await;
    app.connection_type = ConnectionType::Localhost;
    server.clear_calls();

    shift(&mut app, 'P').await;

    assert_eq!(app.modal, Some(Modal::Acl));
    assert_eq!(
        server.calls_to("acl_get"),
        vec![json!({"collection": "users"})]
    );
    assert_eq!(app.acl_state.edit_collection, "users");
    assert_eq!(app.acl_state.edit_rules.len(), 1);
    let rule = &app.acl_state.edit_rules[0];
    assert_eq!(rule.principal_value, "abc");
    assert!(rule.read && !rule.write && !rule.admin);
    let screen = render_to_string(&app, 120, 36);
    assert!(screen.contains("abc"), "{}", screen);
}

#[tokio::test]
async fn test_shift_p_refuses_uneditable_rule() {
    let server = two_collections();
    server.respond(
        "acl_get",
        json!({
            "collection": "users",
            "rules": [{
                "principal": {"type": "anyone"},
                "permissions": {"read": true, "write": false, "admin": false}
            }]
        }),
    );
    let mut app = app_with_server(&server).await;
    app.connection_type = ConnectionType::Localhost;

    shift(&mut app, 'P').await;

    assert_eq!(app.modal, None);
    let error = app.error_message.clone().unwrap_or_default();
    assert!(
        error.contains("contains rules this editor cannot edit yet"),
        "{}",
        error
    );
    assert!(error.contains("users"), "{}", error);
}

#[tokio::test]
async fn test_shift_p_reports_acl_get_error_and_does_not_open() {
    let server = two_collections();
    server.fail("acl_get", "permission denied");
    let mut app = app_with_server(&server).await;
    app.connection_type = ConnectionType::Localhost;

    shift(&mut app, 'P').await;

    assert_eq!(app.modal, None);
    let error = app.error_message.clone().unwrap_or_default();
    assert!(error.contains("ACL betoltese sikertelen"), "{}", error);
    assert!(error.contains("permission denied"), "{}", error);
}

#[tokio::test]
async fn test_shift_p_needs_localhost() {
    let server = two_collections();
    let mut app = app_with_server(&server).await;
    // Unknown (stdio) counts as localhost; a remote URL does not
    assert!(app.connection_type.is_localhost());
    app.connection_type = ConnectionType::External;
    server.clear_calls();

    shift(&mut app, 'P').await;

    assert_eq!(app.modal, None);
    assert_eq!(
        app.error_message.as_deref(),
        Some("ACL editing requires localhost connection")
    );
    assert!(server.calls().is_empty());
}

// === Database switch (HTTP: db_open) ===

async fn open_database_modal_with_path(app: &mut App, path: &str) {
    press(app, KeyCode::Char('a')).await;
    press(app, KeyCode::Char('b')).await;
    assert_eq!(app.modal, Some(Modal::Database));
    assert!(app.database_state.is_http_mode);
    // The path input may be prefilled with the current database
    while !app.database_state.path.is_empty() {
        press(app, KeyCode::End).await;
        press(app, KeyCode::Backspace).await;
    }
    type_text(app, path).await;
    assert_eq!(app.database_state.path, path);
}

#[tokio::test]
async fn test_database_switch_reports_success() {
    let server = two_collections();
    server.respond("db_open", json!({"success": true}));
    let mut app = app_with_server(&server).await;
    open_database_modal_with_path(&mut app, "/data/New.mlite").await;
    server.clear_calls();

    press(&mut app, KeyCode::Enter).await;

    assert_eq!(app.modal, None);
    assert_eq!(
        server.calls_to("db_open"),
        vec![json!({"path": "/data/New.mlite", "create": false})]
    );
    assert_eq!(app.error_message, None);
    assert_eq!(
        app.status_message.as_deref(),
        Some("Adatbazis megnyitva: /data/New.mlite")
    );
}

#[tokio::test]
async fn test_database_switch_keeps_collection_list_error_visible() {
    let server = two_collections();
    server.respond("db_open", json!({"success": true}));
    let mut app = app_with_server(&server).await;
    open_database_modal_with_path(&mut app, "/data/new.mlite").await;
    server.fail("collection_list", "list failed");

    press(&mut app, KeyCode::Enter).await;

    assert_eq!(app.modal, None);
    assert_eq!(server.calls_to("db_open").len(), 1);
    let error = app.error_message.clone().unwrap_or_default();
    assert!(error.contains("Kollekcio betoltes hiba"), "{}", error);
    assert!(error.contains("list failed"), "{}", error);
    // The success status must not replace the error
    assert_eq!(app.status_message, None);
    // The status bar shows the error (cut short by the key hints at this
    // width; `e` opens the full text)
    let screen = render_to_string(&app, 120, 36);
    assert!(screen.contains("Hiba: Koll"), "{}", screen);
    assert!(!screen.contains("Adatbazis megnyitva"), "{}", screen);
}

// === Typing ===

#[tokio::test]
async fn test_j_and_k_go_into_fulltext_query() {
    let server = two_collections();
    server.respond(
        "index_list",
        json!({
            "btree_indexes": [],
            "fulltext_indexes": [{"name": "users_bio_fts", "field": "bio", "language": "english"}]
        }),
    );
    let mut app = app_with_server(&server).await;

    shift(&mut app, 'F').await;
    assert_eq!(app.modal, Some(Modal::Fulltext));
    type_text(&mut app, "jk").await;

    assert_eq!(app.fulltext_state.query, "jk");
    assert_eq!(app.fulltext_state.selected_result, 0);
}

#[tokio::test]
async fn test_uppercase_j_and_c_go_into_export_path() {
    let server = two_collections();
    let mut app = app_with_server(&server).await;
    press(&mut app, KeyCode::Char('a')).await;
    press(&mut app, KeyCode::Char('o')).await;
    assert_eq!(app.modal, Some(Modal::Export));
    assert_eq!(app.export_state.file_path, "users.json");

    type_text(&mut app, "JC").await;

    assert_eq!(app.export_state.file_path, "users.jsonJC");
    assert_eq!(app.modal, Some(Modal::Export));
}

// === Rendering smoke test ===

const ALL_MODALS: [Modal; 21] = [
    Modal::Search,
    Modal::Actions,
    Modal::Help,
    Modal::Confirm,
    Modal::Insert,
    Modal::Index,
    Modal::Query,
    Modal::Export,
    Modal::Filter,
    Modal::ErrorDetail,
    Modal::NewCollection,
    Modal::Script,
    Modal::ServerInfo,
    Modal::Update,
    Modal::Database,
    Modal::ApiKey,
    Modal::Fulltext,
    Modal::Acl,
    Modal::Listener,
    Modal::VectorSearch,
    Modal::Rag,
];

const SIZES: [(u16, u16); 3] = [(120, 36), (80, 24), (20, 6)];

/// The main screen and every modal draw without panic at normal, minimal
/// and absurdly small sizes; without a server and with loaded data
#[tokio::test]
async fn test_main_screen_and_every_modal_render_at_all_sizes() {
    let server = two_collections();
    let connected = app_with_server(&server).await;
    let offline = App::new(Config::default());

    for mut app in [offline, connected] {
        for (width, height) in SIZES {
            app.modal = None;
            let screen = render_to_string(&app, width, height);
            assert_eq!(screen.lines().count(), height as usize);

            for modal in ALL_MODALS {
                app.modal = Some(modal);
                render_to_string(&app, width, height);
            }
        }
    }
}

/// Modals opened the way the user opens them (state initialised by the
/// real open path), at every size
#[tokio::test]
async fn test_modals_opened_by_keys_render_at_all_sizes() {
    let server = two_collections();
    // (keys that open the modal, the modal)
    let openers: Vec<(Vec<(KeyCode, KeyModifiers)>, Modal)> = vec![
        (
            vec![(KeyCode::Char('/'), KeyModifiers::NONE)],
            Modal::Search,
        ),
        (
            vec![(KeyCode::Char('a'), KeyModifiers::NONE)],
            Modal::Actions,
        ),
        (vec![(KeyCode::Char('?'), KeyModifiers::NONE)], Modal::Help),
        (
            vec![(KeyCode::Char('f'), KeyModifiers::NONE)],
            Modal::Filter,
        ),
        (
            vec![(KeyCode::Char('i'), KeyModifiers::NONE)],
            Modal::Insert,
        ),
        (
            vec![
                (KeyCode::Tab, KeyModifiers::NONE),
                (KeyCode::Char('e'), KeyModifiers::NONE),
            ],
            Modal::Insert,
        ),
        (
            vec![
                (KeyCode::Tab, KeyModifiers::NONE),
                (KeyCode::Char('d'), KeyModifiers::NONE),
            ],
            Modal::Confirm,
        ),
        (
            vec![(KeyCode::Char('D'), KeyModifiers::SHIFT)],
            Modal::Confirm,
        ),
        (
            vec![
                (KeyCode::Char('a'), KeyModifiers::NONE),
                (KeyCode::Char('r'), KeyModifiers::NONE),
            ],
            Modal::Query,
        ),
        (
            vec![
                (KeyCode::Char('a'), KeyModifiers::NONE),
                (KeyCode::Char('o'), KeyModifiers::NONE),
            ],
            Modal::Export,
        ),
        (
            vec![
                (KeyCode::Char('a'), KeyModifiers::NONE),
                (KeyCode::Char('c'), KeyModifiers::NONE),
            ],
            Modal::NewCollection,
        ),
        (
            vec![
                (KeyCode::Char('a'), KeyModifiers::NONE),
                (KeyCode::Char('x'), KeyModifiers::NONE),
            ],
            Modal::Index,
        ),
        (
            vec![
                (KeyCode::Char('a'), KeyModifiers::NONE),
                (KeyCode::Char('s'), KeyModifiers::NONE),
            ],
            Modal::Script,
        ),
        (
            vec![
                (KeyCode::Char('a'), KeyModifiers::NONE),
                (KeyCode::Char('b'), KeyModifiers::NONE),
            ],
            Modal::Database,
        ),
        (
            vec![(KeyCode::Char('K'), KeyModifiers::SHIFT)],
            Modal::ApiKey,
        ),
        (vec![(KeyCode::Char('A'), KeyModifiers::SHIFT)], Modal::Acl),
        (
            vec![(KeyCode::Char('L'), KeyModifiers::SHIFT)],
            Modal::Listener,
        ),
        (
            vec![(KeyCode::Char('V'), KeyModifiers::SHIFT)],
            Modal::VectorSearch,
        ),
        (vec![(KeyCode::Char('E'), KeyModifiers::SHIFT)], Modal::Rag),
    ];

    for (keys, modal) in openers {
        let mut app = app_with_server(&server).await;
        for (code, modifiers) in &keys {
            press_with(&mut app, *code, *modifiers).await;
        }
        assert_eq!(app.modal, Some(modal), "keys: {:?}", keys);
        for (width, height) in SIZES {
            render_to_string(&app, width, height);
        }
    }
}
