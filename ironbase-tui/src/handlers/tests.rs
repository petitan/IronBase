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
use ironbase_mcp_types::responses::{
    FulltextIndexInfo, FuzzyIndexInfo, HnswParams, IndexListAll, IndexListResponse,
    IndexStatsResponse, TypedIndexList, VectorIndexInfo, VectorMetric,
};
use serde_json::{json, to_value};
use std::sync::Arc;

/// "users" (30 docs) and "visits" (5 docs)
fn two_collections() -> Arc<FakeServer> {
    FakeServer::new(vec![
        ("users", numbered_docs(30)),
        ("visits", numbered_docs(5)),
    ])
}

/// An `index_list` answer without `type`, as the server serializes it
fn index_list(
    btree: Vec<&str>,
    fulltext: Vec<FulltextIndexInfo>,
    fuzzy: Vec<FuzzyIndexInfo>,
    vector: Vec<VectorIndexInfo>,
) -> serde_json::Value {
    let btree = btree.into_iter().map(String::from).collect();
    to_value(IndexListResponse::All(IndexListAll::new(
        btree, fulltext, fuzzy, vector,
    )))
    .unwrap()
}

/// A 3-dimensional vector index with 2 vectors
fn vector_index(name: &str, field: &str, metric: VectorMetric) -> VectorIndexInfo {
    VectorIndexInfo {
        name: name.into(),
        field: field.into(),
        dim: 3,
        metric,
        max_vectors: 100_000,
        vector_count: 2,
        cache_file: format!("{name}.hnsw"),
        created_at: "2026-11-14T10:00:00Z".into(),
        hnsw_params: HnswParams {
            m: 16,
            ef_construction: 200,
            ef_search: 50,
        },
    }
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

    // The same document again: the server deletes nothing, and says so
    app.documents.insert(1, json!({"_id": 1}));
    press(&mut app, KeyCode::Char('d')).await;
    press(&mut app, KeyCode::Char('y')).await;
    assert_eq!(
        app.error_message.as_deref(),
        Some("Dokumentum nem található")
    );
}

// === Index list ===

#[tokio::test]
async fn test_index_list_drop_flow() {
    let server = two_collections();
    server.respond(
        "index_list",
        index_list(vec!["users_id", "users_age"], vec![], vec![], vec![]),
    );
    server.respond(
        "index_stats",
        to_value(IndexStatsResponse::BTree(TypedIndexList::new(vec![]))).unwrap(),
    );
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
    assert_eq!(app.index_state.error, None);
    assert_eq!(
        app.index_state.message.as_deref(),
        Some("Index törölve: users_age")
    );
}

// === Confirmed actions: the call uses the names captured when the confirm opened ===

#[tokio::test]
async fn test_vector_index_drop_executes_with_captured_names() {
    let server = two_collections();
    let mut app = app_with_server(&server).await;
    // Scripted after loading: only the vector modal asks `index_list` with `type: vector`
    server.respond(
        "index_list",
        to_value(IndexListResponse::Vector(TypedIndexList::new(vec![
            vector_index("users_emb_vec", "emb", VectorMetric::Cosine),
        ])))
        .unwrap(),
    );

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
    assert_eq!(app.vector_state.error, None);
    assert_eq!(
        app.vector_state.message.as_deref(),
        Some("Index 'users_emb_vec' deleted!")
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
        index_list(
            vec!["users_bio_fts", "users_id"],
            vec![FulltextIndexInfo {
                name: "users_bio_fts".into(),
                field: "bio".into(),
                language: "english".into(),
                min_word_length: 2,
                accent_folding: true,
                num_documents: 0,
                num_tokens: 0,
            }],
            vec![],
            vec![],
        ),
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

// === Read-side responses: the server's real shapes (mcp-server/tests/golden/response_wire.txt) ===

/// `index_list` without `type`, as the server sends it (golden "all types")
const INDEX_LIST_ALL: &str = r#"{"btree_indexes":["users_age","users_bio_fts","users_city_age","users_email","users_id","users_name_fuzzy","users_vec_emb"],"fulltext_indexes":[{"name":"users_bio_fts","field":"bio","language":"hungarian","min_word_length":2,"accent_folding":true,"num_documents":3,"num_tokens":6}],"fuzzy_indexes":[{"name":"users_name_fuzzy","field":"name","algorithm":"jarowinkler","threshold":0.8,"num_entries":4}],"vector_indexes":[{"name":"users_vec_emb","field":"emb","dim":3,"metric":"dotproduct","max_vectors":100000,"vector_count":3,"cache_file":"users_vec_emb.hnsw","created_at":"2026-11-14T10:00:00Z","hnsw_params":{"m":16,"ef_construction":200,"ef_search":50}}],"summary":{"btree_count":7,"fulltext_count":1,"fuzzy_count":1,"vector_count":1,"total":10}}"#;

fn wire(text: &str) -> serde_json::Value {
    serde_json::from_str(text).unwrap()
}

/// The line of `screen` that contains `needle`
fn line_with<'a>(screen: &'a str, needle: &str) -> &'a str {
    screen
        .lines()
        .find(|l| l.contains(needle))
        .unwrap_or_else(|| panic!("no line with {needle:?} in:\n{screen}"))
}

#[tokio::test]
async fn test_explain_modal_shows_real_plan_fields() {
    let server = two_collections();
    server.respond(
        "explain",
        wire(
            r#"{"plan":{"chosenPlan":{"queryPlan":"IndexScan","indexUsed":"users_email","field":"email","stage":"FETCH_WITH_INDEX","indexType":"equality","searchKey":"String(\"bea@example.com\")","estimatedCost":"O(log n)"},"selectionReason":"Index users_email on field email (selectivity: 0.2500, method: uniform, est. rows: 1)","estimatedRows":33.333333333333336,"candidates":[],"candidateCount":1}}"#,
        ),
    );
    let mut app = app_with_server(&server).await;

    press(&mut app, KeyCode::Char('a')).await;
    press(&mut app, KeyCode::Char('r')).await;
    assert_eq!(app.modal, Some(Modal::Query));
    press_with(&mut app, KeyCode::Char('e'), KeyModifiers::CONTROL).await;

    assert_eq!(server.calls_to("explain").len(), 1);
    let screen = render_to_string(&app, 140, 45);
    assert!(
        line_with(&screen, "Plan:").contains("IndexScan"),
        "{screen}"
    );
    assert!(
        line_with(&screen, "Index:").contains("users_email"),
        "{screen}"
    );
    let rows_line = line_with(&screen, "Est. rows:");
    assert!(rows_line.contains("33"), "{screen}");
    assert!(!rows_line.contains("33.3"), "{screen}");
    assert!(
        line_with(&screen, "Stage:").contains("FETCH_WITH_INDEX"),
        "{screen}"
    );
    assert!(line_with(&screen, "Cost:").contains("O(log n)"), "{screen}");
    assert!(!screen.contains("Unknown"), "{screen}");
}

#[tokio::test]
async fn test_server_info_shows_index_totals() {
    let server = two_collections();
    server.respond(
        "db_stats",
        wire(
            r#"{"database":{"path":"/data/test.mlite","file_size_bytes":71483,"file_size":"69.81 KB"},"collections":[{"name":"users","document_count":4,"btree_index_count":7,"btree_indexes":["users_age","users_bio_fts","users_city_age","users_email","users_id","users_name_fuzzy","users_vec_emb"],"vector_index_count":1,"vector_indexes":[{"name":"users_vec_emb","field":"emb","dim":3,"metric":"dotproduct","vector_count":3}],"total_vectors":3}],"summary":{"collection_count":1,"total_documents":4,"total_btree_indexes":7,"total_vector_indexes":1,"total_vectors":3},"server":{"version":"1.0.0","protocol":"http","host":"127.0.0.1","port":8080,"require_api_key":true},"storage_wastage":{"file_size_bytes":71483,"file_size_mb":0,"estimated_live_bytes":0,"estimated_live_mb":0,"bloat_ratio":"infinity (never compacted)","total_writes":4,"total_live":4,"dead_writes":0}}"#,
        ),
    );
    let mut app = app_with_server(&server).await;

    shift(&mut app, 'I').await;

    assert_eq!(app.modal, Some(Modal::ServerInfo));
    let screen = render_to_string(&app, 140, 45);
    assert!(
        line_with(&screen, "Osszes index:").contains('7'),
        "{screen}"
    );
    assert!(
        line_with(&screen, "ebbol vektor:").contains("1 (3 vektor)"),
        "{screen}"
    );
    assert!(
        line_with(&screen, "Szerver:").contains("http://127.0.0.1:8080"),
        "{screen}"
    );
}

#[tokio::test]
async fn test_index_list_shows_fuzzy_and_vector_indexes() {
    let server = two_collections();
    server.respond("index_list", wire(INDEX_LIST_ALL));
    let mut app = app_with_server(&server).await;

    press(&mut app, KeyCode::Char('a')).await;
    press(&mut app, KeyCode::Char('x')).await;

    assert_eq!(app.modal, Some(Modal::Index));
    let screen = render_to_string(&app, 140, 45);
    let fuzzy = line_with(&screen, "users_name_fuzzy");
    assert!(fuzzy.contains("[FZ]"), "{fuzzy}");
    assert!(
        fuzzy.contains("name (jarowinkler, 0.8, 4 entries)"),
        "{fuzzy}"
    );
    let vector = line_with(&screen, "users_vec_emb");
    assert!(vector.contains("[VEC]"), "{vector}");
    assert!(vector.contains("emb (3d, dot_product, 3 vecs)"), "{vector}");
}

#[tokio::test]
async fn test_vector_index_list_shows_dim_and_metric() {
    let server = two_collections();
    let mut app = app_with_server(&server).await;
    // `index_list` with `type: vector`, as the server sends it (golden "vector")
    server.respond(
        "index_list",
        wire(
            r#"{"indexes":[{"name":"users_vec_emb","field":"emb","dim":3,"metric":"dotproduct","max_vectors":100000,"vector_count":3,"cache_file":"users_vec_emb.hnsw","created_at":"2026-11-14T10:00:00Z","hnsw_params":{"m":16,"ef_construction":200,"ef_search":50}}],"count":1}"#,
        ),
    );

    shift(&mut app, 'V').await;
    press(&mut app, KeyCode::Tab).await;
    press(&mut app, KeyCode::Tab).await;
    assert_eq!(app.vector_state.active_tab, VectorTab::ListIndexes);

    let screen = render_to_string(&app, 140, 45);
    let line = line_with(&screen, "users_vec_emb");
    assert!(
        line.contains("users_vec_emb | emb | dim:3 | dot_product | 3 vectors"),
        "{line}"
    );
}

// === Write-side responses: the server's real shapes (mcp-server/tests/golden/response_wire.txt) ===

/// Open the editor on the selected (first) document of "users", change nothing
async fn open_edit_of_first_user(app: &mut App) {
    if app.active_pane != Pane::Documents {
        press(app, KeyCode::Tab).await;
    }
    press(app, KeyCode::Char('e')).await;
    assert_eq!(app.modal, Some(Modal::Insert));
    assert!(app.insert.is_edit_mode());
}

#[tokio::test]
async fn test_edit_reports_updated_unchanged_and_not_found() {
    let server = two_collections();
    let mut app = app_with_server(&server).await;

    // A real change (built-in update_one)
    open_edit_of_first_user(&mut app).await;
    app.insert.lines = vec![r#"{"_id": 0, "n": 100, "even": true}"#.to_string()];
    press_with(&mut app, KeyCode::Char('s'), KeyModifiers::CONTROL).await;
    assert_eq!(app.status_message.as_deref(), Some("Dokumentum frissítve"));
    assert_eq!(
        server.calls_to("update_one"),
        vec![json!({"collection": "users", "filter": {"_id": 0},
                    "update": {"$set": {"_id": 0, "n": 100, "even": true}}, "upsert": false})]
    );

    // Saved without a change: matched, not modified (golden "matched, not modified")
    server.respond(
        "update_one",
        wire(r#"{"matched_count":1,"modified_count":0}"#),
    );
    open_edit_of_first_user(&mut app).await;
    press_with(&mut app, KeyCode::Char('s'), KeyModifiers::CONTROL).await;
    assert_eq!(
        app.status_message.as_deref(),
        Some("Dokumentum változatlan (nincs módosítás)")
    );

    // Deleted meanwhile (golden "no match")
    server.respond(
        "update_one",
        wire(r#"{"matched_count":0,"modified_count":0}"#),
    );
    open_edit_of_first_user(&mut app).await;
    press_with(&mut app, KeyCode::Char('s'), KeyModifiers::CONTROL).await;
    assert_eq!(
        app.status_message.as_deref(),
        Some("Dokumentum nem található")
    );
}

#[tokio::test]
async fn test_insert_reports_inserted_id_as_sent() {
    let server = two_collections();
    // golden "no _id": the server reports every `_id` as a string
    server.respond("insert_one", wire(r#"{"inserted_id":"31"}"#));
    let mut app = app_with_server(&server).await;

    press(&mut app, KeyCode::Char('i')).await;
    assert_eq!(app.modal, Some(Modal::Insert));
    press_with(&mut app, KeyCode::Char('s'), KeyModifiers::CONTROL).await;

    assert_eq!(server.calls_to("insert_one").len(), 1);
    assert_eq!(app.modal, None);
    assert_eq!(
        app.status_message.as_deref(),
        Some("Dokumentum beszúrva, _id: 31")
    );
}

#[tokio::test]
async fn test_insert_into_fake_server_stores_the_document() {
    let server = two_collections();
    let mut app = app_with_server(&server).await;

    press(&mut app, KeyCode::Char('i')).await;
    app.insert.lines = vec![r#"{"_id": "k-1", "n": 7}"#.to_string()];
    press_with(&mut app, KeyCode::Char('s'), KeyModifiers::CONTROL).await;

    assert_eq!(
        app.status_message.as_deref(),
        Some("Dokumentum beszúrva, _id: k-1")
    );
    assert_eq!(app.collections[0].doc_count, Some(31));
}

/// Actions -> x (index modal), then `n` (create form) with `field` typed in
async fn open_index_create_form(app: &mut App, field: &str) {
    press(app, KeyCode::Char('a')).await;
    press(app, KeyCode::Char('x')).await;
    assert_eq!(app.modal, Some(Modal::Index));
    press(app, KeyCode::Char('n')).await;
    assert!(app.index_state.is_creating);
    type_text(app, field).await;
}

#[tokio::test]
async fn test_index_create_reports_the_server_index_name() {
    let server = two_collections();
    server.respond(
        "index_stats",
        to_value(IndexStatsResponse::BTree(TypedIndexList::new(vec![]))).unwrap(),
    );
    // golden "btree single"
    server.respond(
        "index_create",
        wire(r#"{"index_name":"users_n","field":"n","unique":false,"sparse":false}"#),
    );
    let mut app = app_with_server(&server).await;

    open_index_create_form(&mut app, "n").await;
    press(&mut app, KeyCode::Enter).await;

    assert_eq!(app.index_state.error, None);
    assert_eq!(
        app.index_state.message.as_deref(),
        Some("Index: n létrehozva (users_n)")
    );
    assert_eq!(
        server.calls_to("index_create"),
        vec![json!({"type": "btree", "collection": "users", "field": "n",
                    "unique": false, "sparse": false})]
    );
}

#[tokio::test]
async fn test_index_create_rejects_a_response_of_another_tool() {
    let server = two_collections();
    server.respond(
        "index_stats",
        to_value(IndexStatsResponse::BTree(TypedIndexList::new(vec![]))).unwrap(),
    );
    // Not an `index_create` shape (an `index_drop` answer)
    server.respond(
        "index_create",
        wire(r#"{"success":true,"dropped":"users_n"}"#),
    );
    let mut app = app_with_server(&server).await;

    open_index_create_form(&mut app, "n").await;
    press(&mut app, KeyCode::Enter).await;

    assert_eq!(app.index_state.message, None);
    let error = app.index_state.error.clone().unwrap_or_default();
    assert!(error.contains("index_create"), "{error}");
}

#[tokio::test]
async fn test_vector_index_create_reports_name_or_rejects_bad_response() {
    use crate::state::vector::VectorTab;
    let server = two_collections();
    let mut app = app_with_server(&server).await;

    shift(&mut app, 'V').await;
    press(&mut app, KeyCode::Tab).await;
    assert_eq!(app.vector_state.active_tab, VectorTab::CreateIndex);
    type_text(&mut app, "emb").await;
    app.vector_state.dimension = 3;

    // A response without `index_name`: an error, not "Index '' created!"
    server.respond("index_create", wire(r#"{"success":true}"#));
    press(&mut app, KeyCode::Enter).await;
    assert_eq!(app.vector_state.message, None);
    assert!(app.vector_state.error.is_some());

    // golden "vector defaults" shape, from the built-in answer
    let server = two_collections();
    let mut app = app_with_server(&server).await;
    shift(&mut app, 'V').await;
    press(&mut app, KeyCode::Tab).await;
    type_text(&mut app, "emb").await;
    app.vector_state.dimension = 3;
    press(&mut app, KeyCode::Enter).await;
    assert_eq!(app.vector_state.error, None);
    assert_eq!(
        app.vector_state.message.as_deref(),
        Some("Index 'users_vec_emb' created!")
    );
    assert_eq!(
        server.calls_to("index_create"),
        vec![
            json!({"type": "vector", "collection": "users", "field": "emb", "dim": 3,
                    "metric": "cosine", "max_vectors": 100000, "m": 16,
                    "ef_construction": 200, "ef_search": 50})
        ]
    );
}

#[tokio::test]
async fn test_index_analyze_and_collection_create_drop_use_built_ins() {
    let server = two_collections();
    server.respond(
        "index_stats",
        to_value(IndexStatsResponse::BTree(TypedIndexList::new(vec![]))).unwrap(),
    );
    let mut app = app_with_server(&server).await;

    press(&mut app, KeyCode::Char('a')).await;
    press(&mut app, KeyCode::Char('x')).await;
    press(&mut app, KeyCode::Char('a')).await;
    assert_eq!(
        server.calls_to("index_stats_refresh"),
        vec![json!({"collection": "users"})]
    );
    assert_eq!(app.index_state.error, None);
    assert_eq!(
        app.index_state.message.as_deref(),
        Some("Index statistics frissítve!")
    );
    press(&mut app, KeyCode::Esc).await;

    app.open_new_collection();
    type_text(&mut app, "logs").await;
    press(&mut app, KeyCode::Enter).await;
    assert_eq!(
        server.calls_to("collection_create"),
        vec![json!({"collection": "logs"})]
    );
    assert_eq!(
        app.status_message.as_deref(),
        Some("Kollekció létrehozva: logs")
    );
    assert_eq!(app.current_collection_name(), Some("logs"));
}
