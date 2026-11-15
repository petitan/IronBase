//! Behaviour pins: real key events through `handle_key` against the fake
//! MCP server, asserting on the call log and the App state / rendered screen

use crate::app::{App, Modal, Pane};
use crate::config::Config;
use crate::modals::confirm::ConfirmOption;
use crate::state::rag::RagTab;
use crate::state::vector::VectorTab;
use crate::state::{ConnectionType, ScriptConfirmAction, ScriptMode};
use crate::testing::{
    app_with_server, model_info, numbered_docs, press, press_with, render_to_string, type_text,
    FakeServer,
};
use crossterm::event::{KeyCode, KeyModifiers};
use ironbase_mcp_types::responses::{
    AutoEmbedDisableResponse, EmbedCacheClearResponse, EmbedJobCancelResponse,
    EmbedJobListResponse, EmbedModelsListResponse, EmbedTextResponse, Flag, FulltextIndexInfo,
    FuzzyIndexInfo, HnswParams, IndexListAll, IndexListResponse, IndexStatsResponse, JobInfo,
    JobStatus, JobType, RagCollectionCreateResponse, RagCollectionStatsResponse, RagConfigView,
    RagCreatedConfig, RagCreatedIndexes, RagDocumentImportResponse, RagImported, RagStats,
    TypedIndexList, VectorIndexInfo, VectorMetric,
};
use serde_json::{json, to_value, Value};
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

/// An `index_list` answer with `type: fulltext` (golden "fulltext"): one
/// index per (name, field)
fn fulltext_index_list(indexes: &[(&str, &str)]) -> serde_json::Value {
    let indexes = indexes
        .iter()
        .map(|(name, field)| FulltextIndexInfo {
            name: name.to_string(),
            field: field.to_string(),
            language: "english".into(),
            min_word_length: 2,
            accent_folding: true,
            num_documents: 0,
            num_tokens: 0,
        })
        .collect();
    to_value(IndexListResponse::Fulltext(TypedIndexList::new(indexes))).unwrap()
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
        json!({"scripts": [script_info("cleanup", 3)], "count": 1}),
    );
    server.respond(
        "script_get",
        json!({
            "name": "cleanup", "code": "1 + 1", "description": null,
            "created_at": "2026-11-15T10:00:00Z", "updated_at": "2026-11-15T10:00:00Z",
            "version": 3, "tags": [], "dependencies": []
        }),
    );
    server.respond(
        "script_history",
        json!({
            "history": [script_version("cleanup", 3, "a"), script_version("cleanup", 2, "b")],
            "count": 2
        }),
    );
    server.respond(
        "script_rollback",
        json!({"success": true, "name": "cleanup", "new_version": 4}),
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
    server.respond(
        "auto_embed_disable",
        to_value(AutoEmbedDisableResponse {
            success: true,
            collection: "users".to_string(),
            message: "Auto-embedding disabled".to_string(),
        })
        .unwrap(),
    );
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
    server.respond(
        "embed_cache_clear",
        to_value(EmbedCacheClearResponse::Cleared {
            success: Flag,
            entries_cleared: 3,
            message: "Cleared 3 cache entries".to_string(),
        })
        .unwrap(),
    );
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
    assert_eq!(
        app.rag_state.message.as_deref(),
        Some("Cleared 3 cache entries")
    );
}

/// Without a cache on the server nothing is cleared: the server's
/// `success: false` is shown as an error, not as "cleared"
#[tokio::test]
async fn test_embed_cache_clear_without_a_cache_reports_it() {
    let server = two_collections();
    server.respond(
        "embed_cache_clear",
        to_value(EmbedCacheClearResponse::Disabled {
            success: Flag,
            message: "Embedding cache is not enabled".to_string(),
        })
        .unwrap(),
    );
    let mut app = app_with_server(&server).await;
    open_rag_models_tab(&mut app).await;
    press(&mut app, KeyCode::Char('x')).await;
    press(&mut app, KeyCode::Char('y')).await;
    assert_eq!(
        app.rag_state.error.as_deref(),
        Some("Embedding cache is not enabled")
    );
    assert_eq!(app.rag_state.message, None);
}

/// A job of the job tools' shape
fn job(n: u64, status: JobStatus) -> JobInfo {
    JobInfo {
        id: format!("job_{}", n),
        job_type: JobType::EmbedBackfill {
            collection: "users".to_string(),
            provider: "length".to_string(),
        },
        status,
        created_at_secs: 10 - n,
        updated_at_secs: 0,
        items_processed: 1,
        items_total: Some(4),
    }
}

fn running() -> JobStatus {
    JobStatus::Running {
        progress: 25.0,
        message: "Embedding batch 1".to_string(),
    }
}

/// The server's job list (hash-map order), as `embed_job_list` answers it
fn job_list(jobs: Vec<JobInfo>) -> serde_json::Value {
    to_value(EmbedJobListResponse {
        count: jobs.len(),
        active_count: jobs.iter().filter(|j| j.status.is_running()).count(),
        jobs,
    })
    .unwrap()
}

#[tokio::test]
async fn test_embed_job_cancel_executes_with_captured_job_id() {
    let server = two_collections();
    // Hash-map order on the server: the TUI lists the newest first
    server.respond(
        "embed_job_list",
        job_list(vec![job(1, running()), job(2, running())]),
    );
    server.respond(
        "embed_job_cancel",
        to_value(EmbedJobCancelResponse {
            success: true,
            job_id: "job_1".to_string(),
            message: "Job 'job_1' cancelled".to_string(),
            status: None,
        })
        .unwrap(),
    );
    let mut app = app_with_server(&server).await;
    shift(&mut app, 'E').await;
    press_with(&mut app, KeyCode::BackTab, KeyModifiers::SHIFT).await;
    assert_eq!(app.rag_state.active_tab, RagTab::Jobs);
    press(&mut app, KeyCode::Char('r')).await;
    let ids: Vec<&str> = app.rag_state.jobs.iter().map(|j| j.id.as_str()).collect();
    assert_eq!(ids, ["job_2", "job_1"]);
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
        vec![json!({"job_id": "job_1"})]
    );
    assert_eq!(
        app.rag_state.message.as_deref(),
        Some("Job 'job_1' cancelled")
    );
    // The list is reloaded after the cancel
    assert_eq!(server.calls_to("embed_job_list").len(), 1);
}

/// The jobs are shown with their real fields: the state of the `status`
/// object, the collection of the job type, the item counters (all `?` / empty
/// before: the TUI read `job_id`, a string `status`, `progress`, `total`)
#[tokio::test]
async fn test_rag_jobs_tab_shows_the_job_fields() {
    let server = two_collections();
    let failed = JobStatus::Failed {
        error: "Provider 'nope' not available".to_string(),
    };
    server.respond(
        "embed_job_list",
        job_list(vec![job(1, failed), job(2, running())]),
    );
    let mut app = app_with_server(&server).await;
    shift(&mut app, 'E').await;
    press_with(&mut app, KeyCode::BackTab, KeyModifiers::SHIFT).await;
    press(&mut app, KeyCode::Char('r')).await;

    let screen = render_to_string(&app, 120, 36);
    assert!(
        screen.contains("job_2 | running 25% | backfill users 1/4"),
        "{screen}"
    );
    assert!(
        screen.contains("job_1 | failed: Provider 'nope' not available"),
        "{screen}"
    );
}

/// A finished job cannot be cancelled: the server's reason is shown as an
/// error, not "cancelled"
#[tokio::test]
async fn test_embed_job_cancel_of_a_finished_job_reports_it() {
    let server = two_collections();
    server.respond(
        "embed_job_list",
        job_list(vec![job(1, JobStatus::Cancelled)]),
    );
    server.respond(
        "embed_job_cancel",
        to_value(EmbedJobCancelResponse {
            success: false,
            job_id: "job_1".to_string(),
            message: "Job 'job_1' is already in terminal state".to_string(),
            status: Some(JobStatus::Cancelled),
        })
        .unwrap(),
    );
    let mut app = app_with_server(&server).await;
    shift(&mut app, 'E').await;
    press_with(&mut app, KeyCode::BackTab, KeyModifiers::SHIFT).await;
    press(&mut app, KeyCode::Char('r')).await;
    press(&mut app, KeyCode::Char('x')).await;
    press(&mut app, KeyCode::Char('y')).await;
    assert_eq!(
        app.rag_state.error.as_deref(),
        Some("Job 'job_1' is already in terminal state")
    );
}

/// The test embedding and auto-embed use the provider selected on the
/// Models tab (before: always `ollama`, an error on a server without it)
#[tokio::test]
async fn test_rag_models_tab_uses_the_selected_provider() {
    let server = two_collections();
    server.respond(
        "embed_models_list",
        to_value(EmbedModelsListResponse {
            models: vec![model_info("vllm"), model_info("openai")],
            count: 2,
            default_provider: "vllm".to_string(),
        })
        .unwrap(),
    );
    server.respond(
        "embed_text",
        to_value(EmbedTextResponse {
            vector: vec![0.5, 1.0],
            dimension: 2,
            provider: "vllm".to_string(),
            model: "vllm-model".to_string(),
        })
        .unwrap(),
    );
    let mut app = app_with_server(&server).await;
    open_rag_models_tab(&mut app).await;
    // Sorted by name: openai, vllm
    press(&mut app, KeyCode::Char('j')).await;
    press(&mut app, KeyCode::Char('t')).await;
    type_text(&mut app, "hi").await;
    press(&mut app, KeyCode::Enter).await;
    assert_eq!(
        server.calls_to("embed_text"),
        vec![json!({"text": "hi", "provider": "vllm"})]
    );
    let screen = render_to_string(&app, 120, 36);
    assert!(
        screen.contains("Provider: vllm (vllm-model) | Dim: 2 | [0.5000, 1.0000]"),
        "{screen}"
    );

    press(&mut app, KeyCode::Esc).await;
    press(&mut app, KeyCode::Char('e')).await;
    assert_eq!(
        server.calls_to("auto_embed_enable"),
        vec![json!({"collection": "users", "source_field": "content",
            "target_field": "embedding", "provider": "vllm", "skip_if_exists": false})]
    );
}

/// A failing provider list is reported, not shown as "Loading models..."
#[tokio::test]
async fn test_rag_open_reports_a_failing_models_list() {
    let server = two_collections();
    server.fail("embed_models_list", "Embedding not available");
    let mut app = app_with_server(&server).await;
    shift(&mut app, 'E').await;
    assert_eq!(app.modal, Some(Modal::Rag));
    assert!(
        app.rag_state
            .error
            .as_deref()
            .is_some_and(|e| e.contains("Embedding not available")),
        "{:?}",
        app.rag_state.error
    );
}

// === Script modal's own confirm ===

/// A `script_list` entry as the server sends it
fn script_info(name: &str, version: u32) -> Value {
    json!({
        "name": name, "description": null, "created_at": "2026-11-15T10:00:00Z",
        "version": version, "tags": [], "execution_count": 0, "last_run_at": null
    })
}

/// A `script_history` entry as the server sends it
fn script_version(name: &str, version: u32, code: &str) -> Value {
    json!({
        "script_name": name, "version": version, "code": code, "description": null,
        "tags": [], "dependencies": [], "created_at": "2026-11-15T10:00:00Z"
    })
}

#[tokio::test]
async fn test_script_delete_confirm_enter_does_not_confirm() {
    let server = two_collections();
    server.respond(
        "script_list",
        json!({"scripts": [script_info("cleanup", 3)], "count": 1}),
    );
    server.respond(
        "script_delete",
        json!({"success": true, "deleted": "cleanup"}),
    );
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
            "_id": 1,
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
            "_id": 1,
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
    assert_eq!(app.database_state.path.value(), path);
}

#[tokio::test]
async fn test_database_switch_reports_success() {
    let server = two_collections();
    server.respond(
        "db_open",
        json!({"success": true, "path": "/data/New.mlite", "message": "Database opened"}),
    );
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
    server.respond(
        "db_open",
        json!({"success": true, "path": "/data/new.mlite", "message": "Database opened"}),
    );
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
    let mut app = app_with_server(&server).await;
    server.respond(
        "index_list",
        fulltext_index_list(&[("users_bio_fts", "bio")]),
    );

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
async fn test_edit_unsets_removed_fields() {
    let server = two_collections();
    let mut app = app_with_server(&server).await;

    // The first user is {"_id": 0, "n": 0, "even": true}: remove `even`
    open_edit_of_first_user(&mut app).await;
    app.insert.lines = vec![r#"{"_id": 0, "n": 0}"#.to_string()];
    press_with(&mut app, KeyCode::Char('s'), KeyModifiers::CONTROL).await;
    assert_eq!(
        server.calls_to("update_one"),
        vec![json!({"collection": "users", "filter": {"_id": 0},
                    "update": {"$set": {"_id": 0, "n": 0}, "$unset": {"even": ""}},
                    "upsert": false})]
    );
}

#[tokio::test]
async fn test_edit_reports_a_failed_re_embed() {
    let server = two_collections();
    // golden "update_one / re-embedding fails": stored, vector stale
    server.respond(
        "update_one",
        wire(r#"{"matched_count":1,"modified_count":1,"auto_embed_error":"provider down"}"#),
    );
    let mut app = app_with_server(&server).await;
    open_edit_of_first_user(&mut app).await;
    press_with(&mut app, KeyCode::Char('s'), KeyModifiers::CONTROL).await;
    let err = app.error_message.clone().unwrap_or_default();
    assert!(
        err.contains("újrabeágyazás") && err.contains("provider down"),
        "{err:?}"
    );
    assert_ne!(app.status_message.as_deref(), Some("Dokumentum frissítve"));
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

// === Search tool group: the server's real shapes (mcp-server/tests/golden/response_wire.txt) ===

/// "docs": three documents with a `bio`, a `name`, a `title`, a `content` and
/// a 2-dimensional `emb`
fn search_docs() -> Arc<FakeServer> {
    FakeServer::new(vec![(
        "docs",
        vec![
            json!({"_id": 1, "name": "Anna", "bio": "Rust developer", "title": "Anna", "content": "Notes on Rust", "emb": [1.0, 0.0], "even": false}),
            json!({"_id": 2, "name": "Bela", "bio": "Python developer", "title": "Bela", "content": "Notes on Python", "emb": [0.6, 0.8], "even": true}),
            json!({"_id": 3, "name": "Cili", "bio": "Gardener", "title": "Cili", "content": "Tomatoes", "emb": [0.0, 1.0], "even": false}),
        ],
    )])
}

/// The fulltext modal open on "users" over the `bio` and `name` fields, as
/// the open path leaves it
fn open_fulltext_modal(app: &mut App) {
    app.fulltext_state =
        crate::app::FulltextState::new("users".into(), vec!["bio".into(), "name".into()]);
    app.modal = Some(Modal::Fulltext);
}

/// A `fulltext_search` answer with `count` hits, as the server sends it
fn fulltext_hits(count: usize) -> serde_json::Value {
    let results: Vec<String> = (0..count)
        .map(|i| {
            format!(
                r#"{{"_id":{i},"bio":"Rust developer {i}","_score":{}.5,"_matched_tokens":["rust"]}}"#,
                count - i
            )
        })
        .collect();
    wire(&format!(
        r#"{{"results":[{}],"count":{count},"match_scope":"chunk"}}"#,
        results.join(",")
    ))
}

#[tokio::test]
async fn test_fulltext_fields_come_from_the_index_field_not_its_name() {
    let server = two_collections();
    let mut app = app_with_server(&server).await;
    // A fulltext index whose name does not follow `{collection}_{field}_fts`
    server.respond(
        "index_list",
        fulltext_index_list(&[("legacy_body_index", "body"), ("users_bio_fts", "bio")]),
    );

    shift(&mut app, 'F').await;

    assert_eq!(app.modal, Some(Modal::Fulltext), "{:?}", app.error_message);
    assert_eq!(app.fulltext_state.indexed_fields, ["body", "bio"]);
    assert_eq!(app.fulltext_state.field, "body");
    assert_eq!(
        server.calls_to("index_list").last().unwrap()["type"],
        "fulltext"
    );
}

#[tokio::test]
async fn test_fulltext_open_shows_why_the_index_listing_failed() {
    let server = two_collections();
    let mut app = app_with_server(&server).await;
    server.fail("index_list", "connection reset by peer");

    shift(&mut app, 'F').await;

    assert_eq!(app.modal, None);
    let error = app.error_message.clone().unwrap_or_default();
    assert!(error.contains("connection reset by peer"), "{error}");
    assert!(!error.contains("Nincs fulltext index"), "{error}");

    // An empty listing is "no index"
    server.respond("index_list", fulltext_index_list(&[]));
    shift(&mut app, 'F').await;
    assert_eq!(app.modal, None);
    let error = app.error_message.clone().unwrap_or_default();
    assert!(error.contains("Nincs fulltext index"), "{error}");
}

#[tokio::test]
async fn test_fulltext_tab_drops_the_previous_fields_results() {
    let server = two_collections();
    server.respond("fulltext_search", fulltext_hits(2));
    let mut app = app_with_server(&server).await;
    open_fulltext_modal(&mut app);
    type_text(&mut app, "rust").await;
    press(&mut app, KeyCode::Enter).await;
    assert_eq!(app.fulltext_state.results.len(), 2);
    press(&mut app, KeyCode::Down).await;

    press(&mut app, KeyCode::Tab).await;

    assert_eq!(app.fulltext_state.field, "name");
    assert!(app.fulltext_state.results.is_empty());
    assert_eq!(app.fulltext_state.selected_result, 0);
    let screen = render_to_string(&app, 120, 36);
    assert!(screen.contains("Talalatok (0)"), "{screen}");
    assert!(!screen.contains("Rust developer"), "{screen}");
}

#[tokio::test]
async fn test_fulltext_tab_with_a_single_field_keeps_the_results() {
    let server = two_collections();
    server.respond("fulltext_search", fulltext_hits(2));
    let mut app = app_with_server(&server).await;
    app.fulltext_state = crate::app::FulltextState::new("users".into(), vec!["bio".into()]);
    app.modal = Some(Modal::Fulltext);
    type_text(&mut app, "rust").await;
    press(&mut app, KeyCode::Enter).await;
    press(&mut app, KeyCode::Down).await;

    // The field does not change: its hits still belong to it
    press(&mut app, KeyCode::Tab).await;

    assert_eq!(app.fulltext_state.field, "bio");
    assert_eq!(app.fulltext_state.results.len(), 2);
    assert_eq!(app.fulltext_state.selected_result, 1);
}

#[tokio::test]
async fn test_fulltext_search_keeps_a_documents_own_highlights_field() {
    let server = two_collections();
    // golden "fulltext_search / own _highlights, no highlight": the TUI
    // searches without highlight, the document's own `_highlights` is no snippet
    server.respond(
        "fulltext_search",
        wire(
            r#"{"results":[{"_highlights":5,"_id":1,"bio":"Rust developer","_score":1.3862943611198906,"_matched_tokens":["rust"]},{"_id":2,"bio":"Rust tester","_score":0.5,"_matched_tokens":["rust"]}],"count":2,"match_scope":"chunk"}"#,
        ),
    );
    let mut app = app_with_server(&server).await;
    open_fulltext_modal(&mut app);
    type_text(&mut app, "rust").await;
    press(&mut app, KeyCode::Enter).await;

    assert_eq!(app.fulltext_state.error, None);
    assert_eq!(app.fulltext_state.results.len(), 2);
    let hit = &app.fulltext_state.results[0];
    assert_eq!(hit.highlights, None);
    assert_eq!(hit.document["_highlights"], 5);
}

#[tokio::test]
async fn test_fulltext_full_page_says_more_may_exist() {
    let server = two_collections();
    server.respond("fulltext_search", fulltext_hits(50));
    let mut app = app_with_server(&server).await;
    open_fulltext_modal(&mut app);
    type_text(&mut app, "rust").await;
    press(&mut app, KeyCode::Enter).await;

    assert_eq!(server.calls_to("fulltext_search")[0]["limit"], 50);
    let screen = render_to_string(&app, 120, 36);
    assert!(screen.contains("Talalatok (50+)"), "{screen}");

    // Fewer hits than the limit: every match is shown
    server.respond("fulltext_search", fulltext_hits(3));
    press(&mut app, KeyCode::Enter).await;
    let screen = render_to_string(&app, 120, 36);
    assert!(screen.contains("Talalatok (3)"), "{screen}");
    assert!(
        line_with(&screen, "Rust developer 0").contains("3.50"),
        "{screen}"
    );
}

#[tokio::test]
async fn test_fulltext_search_through_the_fake_server() {
    let server = search_docs();
    let mut app = app_with_server(&server).await;
    server.respond(
        "index_list",
        fulltext_index_list(&[("docs_bio_fts", "bio")]),
    );
    shift(&mut app, 'F').await;
    type_text(&mut app, "developer").await;
    press(&mut app, KeyCode::Enter).await;

    assert_eq!(app.fulltext_state.error, None);
    assert_eq!(app.fulltext_state.results.len(), 2);
    assert_eq!(app.fulltext_state.results[0].document["name"], "Anna");
    let screen = render_to_string(&app, 120, 36);
    assert!(
        line_with(&screen, r#"name: "Anna", bio: "Rust developer""#).contains("1.00"),
        "{screen}"
    );
}

/// Open the RAG modal on the selected collection (search tab, query focused)
async fn open_rag_search(app: &mut App) {
    shift(app, 'E').await;
    assert_eq!(app.modal, Some(Modal::Rag));
    assert_eq!(app.rag_state.active_tab, RagTab::Search);
}

#[tokio::test]
async fn test_rag_search_has_no_dead_mode_or_rrf_k_controls() {
    let server = two_collections();
    let mut app = app_with_server(&server).await;
    open_rag_search(&mut app).await;

    let screen = render_to_string(&app, 140, 45);
    assert!(screen.contains(" Limit "), "{screen}");
    assert!(!screen.contains("RRF K"), "{screen}");
    assert!(!screen.contains(" Mode "), "{screen}");

    // Tab: query → limit → query
    press(&mut app, KeyCode::Tab).await;
    assert_eq!(app.rag_state.search_focus, 1);
    press(&mut app, KeyCode::Up).await;
    assert_eq!(app.rag_state.search_limit, 11);
    press(&mut app, KeyCode::Tab).await;
    assert_eq!(app.rag_state.search_focus, 0);

    // The search sends only what `search` takes
    type_text(&mut app, "rust").await;
    press(&mut app, KeyCode::Enter).await;
    let calls = server.calls_to("search");
    assert_eq!(
        calls,
        [
            json!({"collection": "users", "query": "rust", "limit": 11, "format": "structured", "debug": false})
        ]
    );
}

/// A `search` answer with one document that left documents and passages out
fn trimmed_search_response() -> serde_json::Value {
    wire(
        r#"{"verdict":"unknown","documents":[{"doc_id":"b00","title":"Needles","relevance":0.04920634920634921,"passages":[{"text":"needle filler filler"}]}],"count":1,"trimmed":true,"dropped_due_to_budget":2,"dropped_empty_documents":1}"#,
    )
}

#[tokio::test]
async fn test_rag_search_reports_trimmed_and_dropped_results() {
    let server = two_collections();
    server.respond("search", trimmed_search_response());
    let mut app = app_with_server(&server).await;
    open_rag_search(&mut app).await;
    type_text(&mut app, "needle").await;
    press(&mut app, KeyCode::Enter).await;

    let message = app.rag_state.message.clone().unwrap_or_default();
    assert!(message.contains("trimmed"), "{message}");
    assert!(message.contains("2 more"), "{message}");
    assert!(message.contains("1 without body text"), "{message}");
    let screen = render_to_string(&app, 140, 45);
    assert!(screen.contains("Results (1, trimmed)"), "{screen}");
    assert!(
        line_with(&screen, "[Needles] needle filler filler").contains("0.0492"),
        "{screen}"
    );
}

#[tokio::test]
async fn test_rag_search_complete_results_through_the_fake_server() {
    let server = search_docs();
    let mut app = app_with_server(&server).await;
    open_rag_search(&mut app).await;
    type_text(&mut app, "developer").await;
    press(&mut app, KeyCode::Enter).await;

    assert_eq!(app.rag_state.error, None);
    assert_eq!(app.rag_state.message.as_deref(), Some("Found 2 results"));
    let screen = render_to_string(&app, 140, 45);
    assert!(screen.contains("Results (2)"), "{screen}");
    assert!(
        line_with(&screen, "[Anna] Notes on Rust").contains("0.5000"),
        "{screen}"
    );
    assert!(
        line_with(&screen, "[Bela] Notes on Python").contains("0.3333"),
        "{screen}"
    );
}

#[tokio::test]
async fn test_rag_search_rejects_a_malformed_response() {
    let server = two_collections();
    server.respond("search", trimmed_search_response());
    let mut app = app_with_server(&server).await;
    open_rag_search(&mut app).await;
    type_text(&mut app, "x").await;
    press(&mut app, KeyCode::Enter).await;
    // A successful, trimmed search first
    assert_eq!(app.rag_state.search_results.len(), 1);
    assert!(app.rag_state.search_incomplete());

    // `relevance` missing: never defaulted to 0
    server.respond(
        "search",
        wire(
            r#"{"verdict":"unknown","documents":[{"doc_id":"d1","passages":[{"text":"x"}]}],"count":1,"trimmed":false}"#,
        ),
    );
    press(&mut app, KeyCode::Enter).await;

    let error = app.rag_state.error.clone().unwrap_or_default();
    assert!(error.contains("relevance"), "{error}");
    // Nothing of the previous search is left next to the error
    assert!(app.rag_state.search_results.is_empty());
    assert!(!app.rag_state.search_trimmed);
    assert_eq!(app.rag_state.search_dropped_due_to_budget, 0);
    assert_eq!(app.rag_state.search_dropped_empty, 0);
    assert_eq!(app.rag_state.selected_result, 0);
    assert_eq!(app.rag_state.message, None);
    let screen = render_to_string(&app, 140, 45);
    assert!(!screen.contains("trimmed"), "{screen}");
    assert!(!screen.contains("Needles"), "{screen}");
}

/// The vector modal on "docs", the vector `[1.0, 0.0]` typed in
async fn open_vector_search(app: &mut App) {
    shift(app, 'V').await;
    assert_eq!(app.modal, Some(Modal::VectorSearch));
    app.vector_state.search_field = "emb".into();
    type_text(app, "[1.0, 0.0]").await;
}

#[tokio::test]
async fn test_vector_search_shows_scores_in_order() {
    let server = search_docs();
    let mut app = app_with_server(&server).await;
    open_vector_search(&mut app).await;
    press(&mut app, KeyCode::Enter).await;

    assert_eq!(app.vector_state.error, None);
    assert_eq!(app.vector_state.message.as_deref(), Some("Found 3 results"));
    let scores: Vec<Option<f64>> = app.vector_state.results.iter().map(|r| r.score).collect();
    assert_eq!(scores, [Some(1.0), Some(0.6), Some(0.0)]);
    let screen = render_to_string(&app, 140, 45);
    assert!(line_with(&screen, "id:2").contains("0.6000"), "{screen}");
}

#[tokio::test]
async fn test_vector_search_parses_the_projected_score_first_shape() {
    let server = search_docs();
    // golden "vector_search / include projection": `_score` first
    server.respond(
        "vector_search",
        wire(
            r#"{"results":[{"_score":0.9999931454658508,"_id":5,"title":"Gardening"},{"_score":0.9999492168426514,"_id":1,"title":"Rust Handbook"}],"count":2}"#,
        ),
    );
    let mut app = app_with_server(&server).await;
    open_vector_search(&mut app).await;
    press(&mut app, KeyCode::Enter).await;

    assert_eq!(app.vector_state.results.len(), 2);
    assert_eq!(app.vector_state.results[0].score, Some(0.9999931454658508));
    assert_eq!(app.vector_state.results[1].document["_id"], 1);
}

#[tokio::test]
async fn test_vector_search_shows_an_unavailable_score() {
    let server = search_docs();
    // golden "vector_search / own _score, include projection": the document's
    // own `_score` wins; a NaN score is sent as `null`
    server.respond(
        "vector_search",
        wire(
            r#"{"results":[{"_score":"own","_id":2,"heading":"Clash Two"},{"_score":0.9778023958206177,"_id":3,"heading":"Other"},{"_id":1,"heading":"Clash","_score":null}],"count":3}"#,
        ),
    );
    let mut app = app_with_server(&server).await;
    open_vector_search(&mut app).await;
    press(&mut app, KeyCode::Enter).await;

    assert_eq!(app.vector_state.error, None);
    let scores: Vec<Option<f64>> = app.vector_state.results.iter().map(|r| r.score).collect();
    assert_eq!(scores, [None, Some(0.9778023958206177), None]);
    assert_eq!(app.vector_state.results[0].document["_score"], "own");
    let screen = render_to_string(&app, 140, 45);
    assert!(line_with(&screen, "id:2").contains("n/a"), "{screen}");
    assert!(line_with(&screen, "id:3").contains("0.9778"), "{screen}");
    assert!(line_with(&screen, "id:1").contains("n/a"), "{screen}");
}

#[tokio::test]
async fn test_vector_search_filter_is_applied_to_typed_hits() {
    let server = search_docs();
    let mut app = app_with_server(&server).await;
    open_vector_search(&mut app).await;
    app.vector_state.k = 1;
    app.vector_state.filter_input = r#"{"even": false}"#.into();
    press(&mut app, KeyCode::Enter).await;

    // A wider pool is asked for, then filtered and cut to `k`
    assert_eq!(server.calls_to("vector_search")[0]["limit"], 5);
    let ids: Vec<_> = app
        .vector_state
        .results
        .iter()
        .map(|r| r.document["_id"].clone())
        .collect();
    assert_eq!(ids, [json!(1)]);

    // An operator filter is refused, not silently ignored
    app.vector_state.filter_input = r#"{"emb": {"$exists": true}}"#.into();
    press(&mut app, KeyCode::Enter).await;
    let error = app.vector_state.error.clone().unwrap_or_default();
    assert!(error.contains("only equality matches"), "{error}");
}

/// Shift+E, then BackTab `n` times (Tab is text input on Search)
async fn open_rag_tab(app: &mut App, back_tabs: usize, tab: RagTab) {
    shift(app, 'E').await;
    for _ in 0..back_tabs {
        press_with(app, KeyCode::BackTab, KeyModifiers::SHIFT).await;
    }
    assert_eq!(app.rag_state.active_tab, tab);
}

/// `rag_collection_stats` of "users": 6 chunks of 4 documents, provider `length`
fn rag_stats() -> serde_json::Value {
    to_value(RagCollectionStatsResponse {
        collection: "users".to_string(),
        rag_enabled: true,
        config: Some(RagConfigView {
            embedding_field: "embedding".to_string(),
            text_field: "content".to_string(),
            text_fields: vec!["content".to_string()],
            provider: "length".to_string(),
            language: "none".to_string(),
            dimension: 2,
            created_at: "2026-11-14T10:00:00Z".to_string(),
        }),
        stats: RagStats {
            chunk_count: 6,
            source_document_count: 4,
            vector_indexes: vec![],
            fulltext_indexes: vec![],
        },
        provider_info: None,
    })
    .unwrap()
}

/// The counts are read from the nested `stats` and the provider from
/// `config` (before: from the top level, "0 chunks | ?" and "0 source docs")
#[tokio::test]
async fn test_rag_collections_tab_shows_the_nested_stats() {
    let server = two_collections();
    server.respond("rag_collection_stats", rag_stats());
    let mut app = app_with_server(&server).await;
    open_rag_tab(&mut app, 3, RagTab::Collections).await;

    press(&mut app, KeyCode::Char('r')).await;
    assert_eq!(
        server.calls_to("rag_collection_stats"),
        vec![json!({"collection": "users"})]
    );
    let screen = render_to_string(&app, 120, 36);
    assert!(
        screen.contains("users | 6 chunks | 4 docs | length"),
        "{screen}"
    );

    press(&mut app, KeyCode::Enter).await;
    assert_eq!(
        app.rag_state.message.as_deref(),
        Some("users: 6 chunks, 4 source docs")
    );
}

/// The import reports what the server stored, and a failed import does not
/// leave the previous result on screen
#[tokio::test]
async fn test_rag_import_reports_the_stored_chunks() {
    let server = two_collections();
    server.respond(
        "rag_document_import",
        to_value(RagDocumentImportResponse::Imported(RagImported {
            success: true,
            collection: "users".to_string(),
            doc_id: "d1".to_string(),
            chunks_created: 3,
            chunks_replaced: 1,
            dimension: 2,
            provider: "length".to_string(),
            language_ignored: false,
        }))
        .unwrap(),
    );
    let mut app = app_with_server(&server).await;
    open_rag_tab(&mut app, 4, RagTab::Import).await;
    type_text(&mut app, "Hello").await;
    press(&mut app, KeyCode::Enter).await;
    assert_eq!(
        server.calls_to("rag_document_import"),
        vec![
            json!({"collection": "users", "content": "Hello", "chunk_size": 1000,
            "overlap": 100, "mode": "auto", "if_exists": "replace", "language": "none"})
        ]
    );
    assert_eq!(
        app.rag_state.message.as_deref(),
        Some("Imported 'd1': 3 chunks created, 1 replaced")
    );
    assert!(app.rag_state.import_result.is_some());

    server.fail("rag_document_import", "Embedding failed");
    press(&mut app, KeyCode::Enter).await;
    assert!(app.rag_state.import_result.is_none());
    assert!(app
        .rag_state
        .error
        .as_deref()
        .is_some_and(|e| e.contains("Embedding failed")));
}

/// Creating a RAG collection says what already existed
#[tokio::test]
async fn test_rag_create_reports_existing_indexes() {
    let server = two_collections();
    server.respond(
        "rag_collection_create",
        to_value(RagCollectionCreateResponse {
            success: true,
            collection: "kb".to_string(),
            config: RagCreatedConfig {
                embedding_field: "embedding".to_string(),
                text_field: "content".to_string(),
                text_fields: vec!["content".to_string()],
                context_fields: vec![],
                provider: "ollama".to_string(),
                language: "hungarian".to_string(),
                dimension: 768,
            },
            indexes: RagCreatedIndexes {
                collection_created: true,
                vector_created: false,
                fulltext_created: 0,
            },
        })
        .unwrap(),
    );
    let mut app = app_with_server(&server).await;
    open_rag_tab(&mut app, 3, RagTab::Collections).await;
    press(&mut app, KeyCode::Char('c')).await;
    type_text(&mut app, "kb").await;
    press(&mut app, KeyCode::Enter).await;
    assert_eq!(
        server.calls_to("rag_collection_create"),
        vec![json!({"collection": "kb", "embedding_field": "embedding",
            "text_field": "content", "provider": "ollama", "language": "hungarian"})]
    );
    assert_eq!(
        app.rag_state.message.as_deref(),
        Some("RAG collection 'kb' ready: vector index already existed, 0 fulltext index(es) created (768 dims, ollama)")
    );
}

/// The ACL list: a deleted ACL that did not exist is reported (the TUI read
/// `success`, always true, so "not found" never showed), and the list is
/// reloaded after a delete
#[tokio::test]
async fn test_acl_delete_reports_a_missing_acl() {
    let server = two_collections();
    server.respond(
        "acl_list",
        json!({"rules": [{"_id": 1, "collection": "users", "rules": [{
            "principal": {"type": "apikey", "value": "abc"},
            "permissions": {"read": true, "write": false, "admin": false}}]}],
            "count": 1, "note": "Built-in rules (_system.* protection) are not shown here"}),
    );
    server.respond(
        "acl_delete",
        json!({"success": true, "collection": "users", "deleted": false,
            "note": "No custom ACL found for this collection."}),
    );
    let mut app = app_with_server(&server).await;
    app.connection_type = ConnectionType::Localhost;
    shift(&mut app, 'A').await;
    assert_eq!(app.modal, Some(Modal::Acl));
    assert_eq!(app.acl_state.acls.len(), 1);

    server.clear_calls();
    app.acl_state.start_delete();
    press(&mut app, KeyCode::Char('y')).await;
    assert_eq!(app.acl_state.error.as_deref(), Some("ACL not found"));
    // The stale row is reloaded
    assert_eq!(server.calls_to("acl_list").len(), 1);
}

/// Shift+P on a collection without a custom ACL opens an empty editor
#[tokio::test]
async fn test_shift_p_without_a_stored_acl_opens_an_empty_editor() {
    let server = two_collections();
    server.respond(
        "acl_get",
        json!({"collection": "users", "rules": null,
            "note": "No custom ACL for this collection. Default rules apply."}),
    );
    let mut app = app_with_server(&server).await;
    app.connection_type = ConnectionType::Localhost;
    shift(&mut app, 'P').await;
    assert_eq!(app.modal, Some(Modal::Acl));
    assert_eq!(app.acl_state.edit_collection, "users");
    assert!(app.acl_state.edit_rules.is_empty());
}

/// A listener created over an existing one is reported as replaced, with the
/// server's note (a restart applies it); the list is reloaded
#[tokio::test]
async fn test_listener_create_reports_replace_and_restart() {
    let server = two_collections();
    let listeners = json!({"listeners": [{"_id": "web", "bind": "127.0.0.1:9090",
        "tls": false, "enabled": true}], "count": 1, "collection": "_system.listeners",
        "note": "Changes require server restart to take effect"});
    server.respond("listener_list", listeners);
    server.respond(
        "listener_create",
        json!({"success": true, "id": "web", "bind": "127.0.0.1:9091", "tls": false,
            "action": "updated", "note": "Restart server for changes to take effect"}),
    );
    let mut app = app_with_server(&server).await;
    app.connection_type = ConnectionType::Localhost;
    shift(&mut app, 'L').await;
    assert_eq!(app.modal, Some(Modal::Listener));
    assert_eq!(app.listener_state.listeners.len(), 1);
    assert_eq!(app.listener_state.listeners[0].bind, "127.0.0.1:9090");

    app.listener_state.mode = crate::modals::listener::ListenerModalMode::Add;
    app.listener_state.add_id = "web".to_string();
    app.listener_state.add_bind = "127.0.0.1:9091".to_string();
    press(&mut app, KeyCode::Enter).await;
    assert_eq!(
        server.calls_to("listener_create"),
        vec![json!({"id": "web", "bind": "127.0.0.1:9091", "tls": false})]
    );
    assert_eq!(
        app.listener_state.success.as_deref(),
        Some("Listener 'web' replaced. Restart server for changes to take effect")
    );
    assert_eq!(server.calls_to("listener_list").len(), 2);
}

/// A failed reload after a listener change is reported, not ignored
#[tokio::test]
async fn test_listener_disable_reports_a_failed_reload() {
    let server = two_collections();
    server.respond(
        "listener_list",
        json!({"listeners": [{"_id": "web", "bind": "127.0.0.1:9090",
            "tls": false, "enabled": true}], "count": 1, "collection": "_system.listeners",
            "note": "Changes require server restart to take effect"}),
    );
    server.respond(
        "listener_disable",
        json!({"success": true, "id": "web", "disabled": true,
            "note": "Listener disabled. Restart server for changes to take effect."}),
    );
    let mut app = app_with_server(&server).await;
    app.connection_type = ConnectionType::Localhost;
    shift(&mut app, 'L').await;
    server.fail("listener_list", "gone");
    app.listener_state.mode = crate::modals::listener::ListenerModalMode::Confirm;
    app.listener_state.confirm_action = Some(crate::modals::listener::ListenerAction::Disable);
    press(&mut app, KeyCode::Char('y')).await;
    assert_eq!(
        server.calls_to("listener_disable"),
        vec![json!({"id": "web"})]
    );
    assert!(
        app.listener_state
            .error
            .as_deref()
            .is_some_and(|e| e.starts_with("'web': Listener disabled.")
                && e.contains("reloading the listeners failed")
                && e.contains("gone")),
        "{:?}",
        app.listener_state.error
    );
}

#[tokio::test]
async fn test_script_run_shows_the_servers_result_logs_and_time() {
    let server = two_collections();
    server.respond(
        "script_exec",
        json!({"success": true, "result": 10, "logs": ["hi"], "execution_time_ms": 4242}),
    );
    let mut app = app_with_server(&server).await;
    app.script_state.lines = vec!["print(\"hi\"); 10".to_string()];

    app.run_script_async().await;

    let result = app.script_state.result.clone().unwrap();
    assert!(result.success);
    assert_eq!(result.result, json!(10));
    assert_eq!(result.logs, vec!["hi".to_string()]);
    // The script's own run time on the server, not the round trip
    assert_eq!(result.execution_time_ms, 4242);
    assert_eq!(
        server.calls_to("script_exec"),
        vec![json!({"code": "print(\"hi\"); 10"})]
    );
}

#[tokio::test]
async fn test_script_list_with_a_malformed_entry_is_an_error_not_a_guess() {
    let server = two_collections();
    // No version: the list used to show it as v1
    server.respond(
        "script_list",
        json!({"scripts": [{"name": "cleanup"}], "count": 1}),
    );
    let mut app = app_with_server(&server).await;

    app.load_scripts_async().await;

    assert!(app.script_state.scripts.is_empty());
    let error = app.script_state.error.clone().unwrap_or_default();
    assert!(error.contains("Script lista"), "{}", error);
}

#[tokio::test]
async fn test_api_key_revoke_reports_a_failed_list_reload() {
    use crate::modals::api_key::{ApiKeyInfo, ApiKeyModalMode, ConfirmAction};

    let server = two_collections();
    server.respond(
        "admin_apikey_revoke",
        json!({"success": true, "id": 7, "status": "revoked"}),
    );
    server.fail("admin_apikey_list", "list failed");
    let mut app = app_with_server(&server).await;
    app.modal = Some(Modal::ApiKey);
    app.api_key_state.admin_key = Some("adm".to_string());
    app.api_key_state.is_localhost = true;
    app.api_key_state.set_keys(vec![ApiKeyInfo {
        id: 7,
        name: "ci".to_string(),
        key_preview: "ib_abc...wxyz".to_string(),
        created_at: "2026-11-15T10:00:00Z".to_string(),
        enabled: true,
    }]);
    app.api_key_state.mode = ApiKeyModalMode::Confirm;
    app.api_key_state.confirm_action = Some(ConfirmAction::Revoke);

    press(&mut app, KeyCode::Char('y')).await;

    assert_eq!(
        server.calls_to("admin_apikey_revoke"),
        vec![json!({"id": 7, "admin_key": "adm"})]
    );
    // The revoke happened; the stale list is not silently kept as current
    let error = app.api_key_state.error.clone().unwrap_or_default();
    assert!(error.contains("Key 'ci' revoked!"), "{}", error);
    assert!(error.contains("list failed"), "{}", error);
}

#[tokio::test]
async fn test_api_key_revoke_of_a_missing_key_says_so() {
    use crate::modals::api_key::{ApiKeyInfo, ApiKeyModalMode, ConfirmAction};

    let server = two_collections();
    server.respond(
        "admin_apikey_revoke",
        json!({"success": false, "id": 7, "error": "API key not found"}),
    );
    let mut app = app_with_server(&server).await;
    app.modal = Some(Modal::ApiKey);
    app.api_key_state.admin_key = Some("adm".to_string());
    app.api_key_state.is_localhost = true;
    app.api_key_state.set_keys(vec![ApiKeyInfo {
        id: 7,
        name: "ci".to_string(),
        key_preview: "***".to_string(),
        created_at: String::new(),
        enabled: true,
    }]);
    app.api_key_state.mode = ApiKeyModalMode::Confirm;
    app.api_key_state.confirm_action = Some(ConfirmAction::Revoke);

    press(&mut app, KeyCode::Char('y')).await;

    assert_eq!(app.api_key_state.error.as_deref(), Some("Key not found"));
    assert_eq!(app.api_key_state.mode, ApiKeyModalMode::List);
    assert!(server.calls_to("admin_apikey_list").is_empty());
}

// === Text inputs (shared TextInput widget) ===

/// Ctrl chords edit the field (readline) instead of typing their letter:
/// Ctrl+U used to type a "u" into the path
#[tokio::test]
async fn test_database_path_takes_readline_keys() {
    let server = two_collections();
    let mut app = app_with_server(&server).await;
    open_database_modal_with_path(&mut app, "/data/old.mlite").await;

    press_with(&mut app, KeyCode::Char('w'), KeyModifiers::CONTROL).await;
    assert_eq!(app.database_state.path.value(), "");
    type_text(&mut app, "/data/new.mlite").await;
    press_with(&mut app, KeyCode::Char('a'), KeyModifiers::CONTROL).await;
    press(&mut app, KeyCode::Right).await;
    press_with(&mut app, KeyCode::Char('k'), KeyModifiers::CONTROL).await;
    assert_eq!(app.database_state.path.value(), "/");
    press_with(&mut app, KeyCode::Char('u'), KeyModifiers::CONTROL).await;
    assert_eq!(app.database_state.path.value(), "");
}

/// The field shows the text as typed, with the terminal's cursor at the
/// insertion point - no "|" drawn into the text
#[tokio::test]
async fn test_new_collection_name_uses_the_terminal_cursor() {
    use ratatui::backend::TestBackend;
    use ratatui::layout::Position;
    use ratatui::Terminal;

    let server = two_collections();
    let mut app = app_with_server(&server).await;
    app.open_new_collection();
    type_text(&mut app, "users2").await;
    press(&mut app, KeyCode::Left).await;

    let screen = render_to_string(&app, 100, 30);
    let line = screen
        .lines()
        .find(|l| l.contains("Nev:"))
        .unwrap_or_default()
        .to_string();
    assert!(line.contains("Nev: users2"), "{}", line);
    assert!(!line.contains('|'), "{}", line);

    let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
    terminal
        .draw(|frame| crate::ui::render_ui(frame, &app))
        .unwrap();
    let y = screen.lines().position(|l| l.contains("Nev:")).unwrap() as u16;
    // Columns, not bytes: the frame's border characters are multi-byte
    let x = line[..line.find("users2").unwrap()].chars().count() as u16 + 5;
    assert_eq!(terminal.get_cursor_position().unwrap(), Position::new(x, y));
}
