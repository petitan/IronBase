//! Key handlers: rag

use crate::app::{App, OpenModal, RagState};
use crate::i18n::{t, Op};
use crate::runtime::{Slot, Update};
use crate::state::open_state;
use crate::widgets::{push_digits, Input};
use crossterm::event::{KeyCode, KeyModifiers};
use ironbase_mcp_types::params::{
    AutoEmbedEnableParams, AutoEmbedStatusParams, EmbedTextParams, JobListParams, NoParams,
    RagCollectionCreateParams, RagCollectionStatsParams, RagDocumentImportParams, SearchParams,
};
use ironbase_mcp_types::responses::{EmbedCacheStatsResponse, RagCollectionCreateResponse};
use ironbase_mcp_types::tools::{
    AutoEmbedEnable, AutoEmbedStatus, EmbedCacheStats, EmbedJobList, EmbedModelsList, EmbedText,
    RagCollectionCreate, RagCollectionStats, RagDocumentImport, Search,
};

/// Open RAG & Embedding modal
pub(crate) async fn handle_rag_open(app: &mut App) {
    if app.collections.is_empty() {
        app.set_error(t().select_collection_first());
        return;
    }

    let collection = app
        .current_collection_name()
        .unwrap_or_default()
        .to_string();
    if collection.is_empty() {
        app.set_error(t().select_collection_first());
        return;
    }

    app.open_modal = Some(OpenModal::Rag(RagState::new(collection.clone())));

    // Load models + auto-embed status
    if let Some(ref db) = app.db {
        let mut errors = Vec::new();
        match db.call::<EmbedModelsList>(&NoParams {}).await {
            Ok(models) => open_state!(app, Rag).set_models(models.models),
            Err(e) => errors.push(t().failed(Op::LoadModels, &e)),
        }
        match load_auto_embed_status(db, &collection).await {
            Ok(status) => open_state!(app, Rag).auto_embed_config = Some(status),
            Err(e) => errors.push(t().failed(Op::LoadAutoEmbedStatus, &e)),
        }
        if !errors.is_empty() {
            open_state!(app, Rag).error = Some(errors.join("; "));
        }
    }
}

async fn load_auto_embed_status(
    db: &crate::db::DbWrapper,
    collection: &str,
) -> anyhow::Result<ironbase_mcp_types::responses::AutoEmbedStatusResponse> {
    db.call::<AutoEmbedStatus>(&AutoEmbedStatusParams {
        collection: collection.to_string(),
    })
    .await
}

/// Handle RAG modal keys
pub(crate) async fn handle_rag_key_async(app: &mut App, key: KeyCode, modifiers: KeyModifiers) {
    use crate::state::rag::RagTab;

    match (key, modifiers) {
        // Close modal
        (KeyCode::Esc, _) => {
            // If in a sub-mode, go back first
            if open_state!(app, Rag).creating {
                open_state!(app, Rag).creating = false;
                return;
            }
            if open_state!(app, Rag).models_mode == 1 {
                open_state!(app, Rag).models_mode = 0;
                return;
            }
            app.close_modal();
        }

        // Tab navigation (only when not in text input sub-modes)
        (KeyCode::Tab, KeyModifiers::NONE)
            if !open_state!(app, Rag).creating
                && open_state!(app, Rag).models_mode == 0
                && open_state!(app, Rag).active_tab != RagTab::Search
                && open_state!(app, Rag).active_tab != RagTab::Import =>
        {
            open_state!(app, Rag).next_tab();
        }
        (KeyCode::BackTab, _)
            if !open_state!(app, Rag).creating && open_state!(app, Rag).models_mode == 0 =>
        {
            open_state!(app, Rag).prev_tab();
        }

        _ => match open_state!(app, Rag).active_tab {
            RagTab::Search => handle_rag_search_key(app, key, modifiers).await,
            RagTab::Import => handle_rag_import_key(app, key, modifiers).await,
            RagTab::Collections => handle_rag_collections_key(app, key, modifiers).await,
            RagTab::Models => handle_rag_models_key(app, key, modifiers).await,
            RagTab::Jobs => handle_rag_jobs_key(app, key).await,
        },
    }
}

/// Search tab key handler
pub(crate) async fn handle_rag_search_key(app: &mut App, key: KeyCode, modifiers: KeyModifiers) {
    match (key, modifiers) {
        // Execute search
        (KeyCode::Enter, _) if open_state!(app, Rag).search_focus == 0 => {
            if open_state!(app, Rag).search_query.value().trim().is_empty() {
                open_state!(app, Rag).error = Some(t().query_empty().to_string());
                return;
            }

            let args = SearchParams {
                collection: open_state!(app, Rag).collection.clone(),
                query: open_state!(app, Rag).search_query.value().to_string(),
                filter: None,
                limit: open_state!(app, Rag).search_limit,
                format: "structured".to_string(),
                debug: false,
            };

            let Some(db) = app.db.clone() else {
                return;
            };
            // A running write (import, creation) is not replaced by a search
            if app.tasks.writing() {
                return;
            }
            open_state!(app, Rag).is_loading = true;
            open_state!(app, Rag).error = None;

            // In the background: the screen keeps drawing
            app.tasks.spawn(Slot::Modal, async move {
                let response = db.call::<Search>(&args).await.map_err(|e| e.to_string());
                Box::new(move |app: &mut App| {
                    let Some(state) = app.rag_state_mut() else {
                        return;
                    };
                    let result = response.and_then(|response| state.set_search_response(response));
                    state.is_loading = false;
                    match result {
                        Ok(()) => state.message = Some(state.search_summary()),
                        Err(e) => {
                            // The previous search's results and summary are stale
                            state.clear_search_response();
                            state.message = None;
                            state.error = Some(e);
                        }
                    }
                }) as Update
            });
        }

        // Switch focus between query and limit
        (KeyCode::Tab, KeyModifiers::NONE) => {
            open_state!(app, Rag).search_focus = 1 - open_state!(app, Rag).search_focus.min(1);
        }

        // Navigate results (plain j/k belong to the query input)
        (KeyCode::Up, _) | (KeyCode::Char('k'), KeyModifiers::CONTROL)
            if open_state!(app, Rag).search_focus == 0
                && !open_state!(app, Rag).search_results.is_empty() =>
        {
            open_state!(app, Rag).search_result_up();
        }
        (KeyCode::Down, _) | (KeyCode::Char('j'), KeyModifiers::CONTROL)
            if open_state!(app, Rag).search_focus == 0
                && !open_state!(app, Rag).search_results.is_empty() =>
        {
            open_state!(app, Rag).search_result_down();
        }

        // Adjust the limit with Up/Down when it has the focus
        (KeyCode::Up, _) if open_state!(app, Rag).search_focus == 1 => {
            if open_state!(app, Rag).search_limit < 100 {
                open_state!(app, Rag).search_limit += 1;
            }
        }
        (KeyCode::Down, _) if open_state!(app, Rag).search_focus == 1 => {
            if open_state!(app, Rag).search_limit > 1 {
                open_state!(app, Rag).search_limit -= 1;
            }
        }

        // The query input takes the rest
        _ => {
            open_state!(app, Rag).handle_text_input(Input::Key(key, modifiers));
        }
    }
}

/// Import tab key handler
pub(crate) async fn handle_rag_import_key(app: &mut App, key: KeyCode, modifiers: KeyModifiers) {
    match (key, modifiers) {
        // Execute import
        (KeyCode::Enter, _) => {
            if open_state!(app, Rag)
                .import_content
                .text()
                .trim()
                .is_empty()
            {
                open_state!(app, Rag).error = Some(t().content_empty().to_string());
                return;
            }

            let collection = open_state!(app, Rag).collection.clone();
            let content = open_state!(app, Rag).import_content.text();
            let title = if open_state!(app, Rag).import_title.is_empty() {
                None
            } else {
                Some(open_state!(app, Rag).import_title.value().to_string())
            };
            let args = RagDocumentImportParams {
                collection,
                content,
                title,
                metadata: None,
                doc_id: None,
                chunk_size: open_state!(app, Rag).import_chunk_size,
                overlap: open_state!(app, Rag).import_overlap,
                mode: open_state!(app, Rag).import_mode.clone(),
                provider: None,
                if_exists: ironbase_mcp_types::defaults::default_if_exists(),
                language: ironbase_mcp_types::params::default_rag_language(),
                text_fields: None,
                context_fields: None,
            };

            // One import at a time: a second Enter would cancel the first
            if app.tasks.writing() {
                return;
            }
            let Some(db) = app.db.clone() else {
                return;
            };
            open_state!(app, Rag).is_loading = true;
            open_state!(app, Rag).error = None;
            // A failed import must not leave the previous one's result
            open_state!(app, Rag).import_result = None;

            // In the background: chunking and embedding a long text takes a
            // while, the screen keeps drawing and Esc stops it
            app.tasks.spawn_write(async move {
                let result = db.call::<RagDocumentImport>(&args).await;
                Box::new(move |app: &mut App| {
                    let Some(state) = app.rag_state_mut() else {
                        return;
                    };
                    state.is_loading = false;
                    match result {
                        Ok(result) => {
                            state.message = Some(crate::modals::rag::import_result_text(&result));
                            state.import_result = Some(result);
                        }
                        Err(e) => state.error = Some(format!("{}", e)),
                    }
                }) as Update
            });
        }

        // Toggle mode (when mode field focused)
        (KeyCode::Char(' '), _) if open_state!(app, Rag).import_form_field == 4 => {
            open_state!(app, Rag).toggle_import_mode();
        }

        // Next form field
        (KeyCode::Tab, KeyModifiers::NONE) => {
            open_state!(app, Rag).next_import_field();
        }

        // The content and the title
        _ if open_state!(app, Rag).import_form_field <= 1 => {
            open_state!(app, Rag).handle_text_input(Input::Key(key, modifiers));
        }

        // Text input based on focused field
        (KeyCode::Char(c), KeyModifiers::NONE) | (KeyCode::Char(c), KeyModifiers::SHIFT) => {
            match open_state!(app, Rag).import_form_field {
                2 => {
                    let size = open_state!(app, Rag).import_chunk_size;
                    open_state!(app, Rag).import_chunk_size = push_digits(size, &c.to_string());
                }
                3 => {
                    let overlap = open_state!(app, Rag).import_overlap;
                    open_state!(app, Rag).import_overlap = push_digits(overlap, &c.to_string());
                }
                _ => {}
            }
            open_state!(app, Rag).error = None;
        }
        (KeyCode::Backspace, _) => {
            match open_state!(app, Rag).import_form_field {
                2 => {
                    open_state!(app, Rag).import_chunk_size /= 10;
                }
                3 => {
                    open_state!(app, Rag).import_overlap /= 10;
                }
                _ => {}
            }
            open_state!(app, Rag).error = None;
        }

        _ => {}
    }
}

/// Collections tab key handler
pub(crate) async fn handle_rag_collections_key(
    app: &mut App,
    key: KeyCode,
    modifiers: KeyModifiers,
) {
    // If in create form
    if open_state!(app, Rag).creating {
        match (key, modifiers) {
            (KeyCode::Enter, _) => {
                // Create collection
                let name = open_state!(app, Rag)
                    .create_collection_name
                    .value()
                    .trim()
                    .to_string();
                if name.is_empty() {
                    open_state!(app, Rag).error = Some(t().collection_name_required().to_string());
                    return;
                }

                let args = RagCollectionCreateParams {
                    collection: name,
                    embedding_field: open_state!(app, Rag)
                        .create_embedding_field
                        .value()
                        .to_string(),
                    text_field: open_state!(app, Rag).create_text_field.value().to_string(),
                    text_fields: None,
                    context_fields: None,
                    provider: Some(open_state!(app, Rag).create_provider.clone()),
                    language: open_state!(app, Rag).create_language.clone(),
                };

                // One creation at a time: a second Enter would cancel the first
                if app.tasks.writing() {
                    return;
                }
                let Some(db) = app.db.clone() else {
                    return;
                };
                open_state!(app, Rag).is_loading = true;
                open_state!(app, Rag).error = None;

                // In the background (it creates the indexes): Esc stops it
                app.tasks.spawn_write(async move {
                    let result = db.call::<RagCollectionCreate>(&args).await;
                    Box::new(move |app: &mut App| {
                        let Some(state) = app.rag_state_mut() else {
                            return;
                        };
                        state.is_loading = false;
                        match result {
                            Ok(response) => {
                                state.creating = false;
                                state.message = Some(rag_created_message(&response));
                            }
                            Err(e) => state.error = Some(format!("{}", e)),
                        }
                    }) as Update
                });
            }

            // Toggle provider/language
            (KeyCode::Char(' '), _) if open_state!(app, Rag).create_form_field == 3 => {
                open_state!(app, Rag).toggle_create_provider();
            }
            (KeyCode::Char(' '), _) if open_state!(app, Rag).create_form_field == 4 => {
                open_state!(app, Rag).toggle_create_language();
            }

            // Next field
            (KeyCode::Tab, KeyModifiers::NONE) => {
                open_state!(app, Rag).next_create_field();
            }

            // The name and field inputs take the rest
            _ => {
                open_state!(app, Rag).handle_text_input(Input::Key(key, modifiers));
            }
        }
        return;
    }

    // Normal mode
    match (key, modifiers) {
        // Create new
        (KeyCode::Char('c'), _) => {
            open_state!(app, Rag).creating = true;
            open_state!(app, Rag).create_form_field = 0;
            open_state!(app, Rag).create_collection_name.clear();
        }

        // Navigate
        (KeyCode::Up, _) | (KeyCode::Char('k'), _) => open_state!(app, Rag).collection_up(),
        (KeyCode::Down, _) | (KeyCode::Char('j'), _) => open_state!(app, Rag).collection_down(),

        // Stats for selected collection
        (KeyCode::Enter, _) => {
            let state = open_state!(app, Rag);
            let Some(name) = state
                .rag_collections
                .get(state.selected_collection)
                .map(|c| c.collection.clone())
            else {
                return;
            };
            if let Some(ref db) = app.db {
                match load_rag_stats(db, &name).await {
                    Ok(stats) => {
                        open_state!(app, Rag).message = Some(t().rag_stats_line(
                            &stats.collection,
                            &stats.stats.chunk_count,
                            &stats.stats.source_document_count,
                        ));
                        if let Some(entry) = open_state!(app, Rag)
                            .rag_collections
                            .iter_mut()
                            .find(|c| c.collection == name)
                        {
                            *entry = stats;
                        }
                    }
                    Err(e) => {
                        open_state!(app, Rag).error = Some(format!("{}", e));
                    }
                }
            }
        }

        // Refresh
        (KeyCode::Char('r'), _) => {
            let collection = open_state!(app, Rag).collection.clone();
            if let Some(ref db) = app.db {
                match load_rag_stats(db, &collection).await {
                    Ok(stats) => {
                        open_state!(app, Rag).message = Some(t().stats_refreshed().to_string());
                        open_state!(app, Rag).rag_collections = vec![stats];
                        open_state!(app, Rag).selected_collection = 0;
                    }
                    Err(e) => {
                        open_state!(app, Rag).error = Some(format!("{}", e));
                    }
                }
            }
        }

        // Tab navigation
        (KeyCode::Tab, KeyModifiers::NONE) => open_state!(app, Rag).next_tab(),

        _ => {}
    }
}

/// Models tab key handler
pub(crate) async fn handle_rag_models_key(app: &mut App, key: KeyCode, modifiers: KeyModifiers) {
    // If in test mode
    if open_state!(app, Rag).models_mode == 1 {
        match (key, modifiers) {
            (KeyCode::Enter, _) => {
                if open_state!(app, Rag)
                    .embed_test_input
                    .value()
                    .trim()
                    .is_empty()
                {
                    open_state!(app, Rag).error = Some(t().text_empty().to_string());
                    return;
                }

                let Some(db) = app.db.clone() else {
                    return;
                };
                // A running write (import, creation) is not replaced by a test
                if app.tasks.writing() {
                    return;
                }
                let text = open_state!(app, Rag).embed_test_input.value().to_string();
                open_state!(app, Rag).is_loading = true;
                open_state!(app, Rag).error = None;
                open_state!(app, Rag).embed_test_result = None;
                let args = EmbedTextParams {
                    text,
                    provider: open_state!(app, Rag).selected_provider(),
                };

                // In the background: the provider may be slow (a model loads)
                app.tasks.spawn(Slot::Modal, async move {
                    let result = db.call::<EmbedText>(&args).await;
                    Box::new(move |app: &mut App| {
                        let Some(state) = app.rag_state_mut() else {
                            return;
                        };
                        state.is_loading = false;
                        match result {
                            Ok(result) => state.embed_test_result = Some(result),
                            Err(e) => state.error = Some(format!("{}", e)),
                        }
                    }) as Update
                });
            }

            _ => {
                open_state!(app, Rag).handle_text_input(Input::Key(key, modifiers));
            }
        }
        return;
    }

    // Normal mode
    match (key, modifiers) {
        // Navigate models
        (KeyCode::Up, _) | (KeyCode::Char('k'), _) => open_state!(app, Rag).model_up(),
        (KeyCode::Down, _) | (KeyCode::Char('j'), _) => open_state!(app, Rag).model_down(),

        // Enable auto-embed
        (KeyCode::Char('e'), _) => {
            // One request at a time: a second `e` would cancel the first
            if app.tasks.writing() {
                return;
            }
            let Some(db) = app.db.clone() else {
                return;
            };
            let collection = open_state!(app, Rag).collection.clone();
            let args = AutoEmbedEnableParams {
                collection: collection.clone(),
                source_field: "content".to_string(),
                target_field: "embedding".to_string(),
                provider: open_state!(app, Rag).selected_provider(),
                model: None,
                dimension: None,
                skip_if_exists: false,
                chunking: None,
            };
            open_state!(app, Rag).is_loading = true;
            open_state!(app, Rag).error = None;

            // In the background: the server probes the provider first
            app.tasks.spawn_write(async move {
                let response = db.call::<AutoEmbedEnable>(&args).await;
                let status = match response {
                    Ok(_) => Some(load_auto_embed_status(&db, &collection).await),
                    Err(_) => None,
                };
                Box::new(move |app: &mut App| {
                    let Some(state) = app.rag_state_mut() else {
                        return;
                    };
                    state.is_loading = false;
                    let response = match response {
                        Ok(response) => response,
                        Err(e) => {
                            state.error = Some(format!("{}", e));
                            return;
                        }
                    };
                    // What the server started (backfill / re-embed) and
                    // why the vectors may differ, not a bare "enabled"
                    let mut message = response
                        .message
                        .unwrap_or_else(|| t().auto_embed_enabled().to_string());
                    if let Some(job_id) = response.backfill_job_id {
                        message.push_str(&t().job_suffix(&job_id));
                    }
                    if let Some(warning) = response.warning {
                        message.push_str(&t().warning_suffix(&warning));
                    }
                    state.message = Some(message);
                    match status {
                        Some(Ok(status)) => state.auto_embed_config = Some(status),
                        Some(Err(e)) => state.error = Some(t().failed(Op::LoadAutoEmbedStatus, &e)),
                        None => {}
                    }
                }) as Update
            });
        }

        // Disable auto-embed (with confirmation)
        (KeyCode::Char('d'), _) => app.open_disable_auto_embed_confirm(),

        // Cache stats
        (KeyCode::Char('c'), _) => {
            if let Some(ref db) = app.db {
                match db.call::<EmbedCacheStats>(&NoParams {}).await {
                    Ok(stats) => {
                        open_state!(app, Rag).message = Some(match &stats {
                            EmbedCacheStatsResponse::Enabled { .. } => {
                                t().cache_stats_loaded().to_string()
                            }
                            EmbedCacheStatsResponse::Disabled { message, .. } => message.clone(),
                        });
                        open_state!(app, Rag).cache_stats = Some(stats);
                    }
                    Err(e) => {
                        open_state!(app, Rag).error = Some(format!("{}", e));
                    }
                }
            }
        }

        // Clear cache (with confirmation)
        (KeyCode::Char('x'), _) => app.open_clear_embed_cache_confirm(),

        // Test embed
        (KeyCode::Char('t'), _) => {
            open_state!(app, Rag).models_mode = 1;
            open_state!(app, Rag).embed_test_input.clear();
            open_state!(app, Rag).embed_test_result = None;
        }

        // Tab navigation
        (KeyCode::Tab, KeyModifiers::NONE) => open_state!(app, Rag).next_tab(),

        _ => {}
    }
}

/// Jobs tab key handler
pub(crate) async fn handle_rag_jobs_key(app: &mut App, key: KeyCode) {
    match key {
        // Navigate
        KeyCode::Up | KeyCode::Char('k') => open_state!(app, Rag).job_up(),
        KeyCode::Down | KeyCode::Char('j') => open_state!(app, Rag).job_down(),

        // Cancel job (with confirmation)
        KeyCode::Char('x') => app.open_cancel_embed_job_confirm(),

        // Refresh job list
        KeyCode::Char('r') => {
            if let Some(ref db) = app.db {
                match db
                    .call::<EmbedJobList>(&JobListParams { active_only: false })
                    .await
                {
                    Ok(result) => {
                        open_state!(app, Rag).set_jobs(result.jobs);
                        open_state!(app, Rag).message = Some(t().jobs_refreshed().to_string());
                    }
                    Err(e) => {
                        open_state!(app, Rag).error = Some(format!("{}", e));
                    }
                }
            }
        }

        // Tab navigation
        KeyCode::Tab => open_state!(app, Rag).next_tab(),

        _ => {}
    }
}

async fn load_rag_stats(
    db: &crate::db::DbWrapper,
    collection: &str,
) -> anyhow::Result<ironbase_mcp_types::responses::RagCollectionStatsResponse> {
    db.call::<RagCollectionStats>(&RagCollectionStatsParams {
        collection: collection.to_string(),
    })
    .await
}

/// What `rag_collection_create` did: an existing index is kept (the server
/// does not report whether the collection itself existed)
fn rag_created_message(response: &RagCollectionCreateResponse) -> String {
    t().rag_collection_ready(
        &response.collection,
        response.indexes.vector_created,
        response.indexes.fulltext_created,
        response.config.dimension,
        &response.config.provider,
    )
}
