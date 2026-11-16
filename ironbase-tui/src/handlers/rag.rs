//! Key handlers: rag

use crate::app::{App, Modal, RagState};
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
        app.set_error("Valassz ki egy kollekciot elobb!");
        return;
    }

    let collection = app
        .current_collection_name()
        .unwrap_or_default()
        .to_string();
    if collection.is_empty() {
        app.set_error("Valassz ki egy kollekciot elobb!");
        return;
    }

    app.rag_state = RagState::new(collection.clone());
    app.modal = Some(Modal::Rag);

    // Load models + auto-embed status
    if let Some(ref db) = app.db {
        let mut errors = Vec::new();
        match db.call::<EmbedModelsList>(&NoParams {}).await {
            Ok(models) => app.rag_state.set_models(models.models),
            Err(e) => errors.push(format!("Models: {}", e)),
        }
        match load_auto_embed_status(db, &collection).await {
            Ok(status) => app.rag_state.auto_embed_config = Some(status),
            Err(e) => errors.push(format!("Auto-embed status: {}", e)),
        }
        if !errors.is_empty() {
            app.rag_state.error = Some(errors.join("; "));
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
            if app.rag_state.creating {
                app.rag_state.creating = false;
                return;
            }
            if app.rag_state.models_mode == 1 {
                app.rag_state.models_mode = 0;
                return;
            }
            app.close_modal();
        }

        // Tab navigation (only when not in text input sub-modes)
        (KeyCode::Tab, KeyModifiers::NONE)
            if !app.rag_state.creating
                && app.rag_state.models_mode == 0
                && app.rag_state.active_tab != RagTab::Search
                && app.rag_state.active_tab != RagTab::Import =>
        {
            app.rag_state.next_tab();
        }
        (KeyCode::BackTab, _) if !app.rag_state.creating && app.rag_state.models_mode == 0 => {
            app.rag_state.prev_tab();
        }

        _ => match app.rag_state.active_tab {
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
        (KeyCode::Enter, _) if app.rag_state.search_focus == 0 => {
            if app.rag_state.search_query.value().trim().is_empty() {
                app.rag_state.error = Some("Query is empty".to_string());
                return;
            }

            let args = SearchParams {
                collection: app.rag_state.collection.clone(),
                query: app.rag_state.search_query.value().to_string(),
                filter: None,
                limit: app.rag_state.search_limit,
                format: "structured".to_string(),
                debug: false,
            };

            app.rag_state.is_loading = true;
            app.rag_state.error = None;

            if let Some(ref db) = app.db {
                let result = db
                    .call::<Search>(&args)
                    .await
                    .map_err(|e| e.to_string())
                    .and_then(|response| app.rag_state.set_search_response(response));
                app.rag_state.is_loading = false;
                match result {
                    Ok(()) => app.rag_state.message = Some(app.rag_state.search_summary()),
                    Err(e) => {
                        // The previous search's results and summary are stale
                        app.rag_state.clear_search_response();
                        app.rag_state.message = None;
                        app.rag_state.error = Some(e);
                    }
                }
            }
        }

        // Switch focus between query and limit
        (KeyCode::Tab, KeyModifiers::NONE) => {
            app.rag_state.search_focus = 1 - app.rag_state.search_focus.min(1);
        }

        // Navigate results (plain j/k belong to the query input)
        (KeyCode::Up, _) | (KeyCode::Char('k'), KeyModifiers::CONTROL)
            if app.rag_state.search_focus == 0 && !app.rag_state.search_results.is_empty() =>
        {
            app.rag_state.search_result_up();
        }
        (KeyCode::Down, _) | (KeyCode::Char('j'), KeyModifiers::CONTROL)
            if app.rag_state.search_focus == 0 && !app.rag_state.search_results.is_empty() =>
        {
            app.rag_state.search_result_down();
        }

        // Adjust the limit with Up/Down when it has the focus
        (KeyCode::Up, _) if app.rag_state.search_focus == 1 => {
            if app.rag_state.search_limit < 100 {
                app.rag_state.search_limit += 1;
            }
        }
        (KeyCode::Down, _) if app.rag_state.search_focus == 1 => {
            if app.rag_state.search_limit > 1 {
                app.rag_state.search_limit -= 1;
            }
        }

        // The query input takes the rest
        _ => {
            app.rag_state.handle_text_input(key, modifiers);
        }
    }
}

/// Import tab key handler
pub(crate) async fn handle_rag_import_key(app: &mut App, key: KeyCode, modifiers: KeyModifiers) {
    match (key, modifiers) {
        // Execute import
        (KeyCode::Enter, _) => {
            if app.rag_state.import_content.text().trim().is_empty() {
                app.rag_state.error = Some("Content is empty".to_string());
                return;
            }

            let collection = app.rag_state.collection.clone();
            let content = app.rag_state.import_content.text();
            let title = if app.rag_state.import_title.is_empty() {
                None
            } else {
                Some(app.rag_state.import_title.value().to_string())
            };
            let args = RagDocumentImportParams {
                collection,
                content,
                title,
                metadata: None,
                doc_id: None,
                chunk_size: app.rag_state.import_chunk_size,
                overlap: app.rag_state.import_overlap,
                mode: app.rag_state.import_mode.clone(),
                provider: None,
                if_exists: ironbase_mcp_types::defaults::default_if_exists(),
                language: ironbase_mcp_types::params::default_rag_language(),
                text_fields: None,
                context_fields: None,
            };

            app.rag_state.is_loading = true;
            app.rag_state.error = None;
            // A failed import must not leave the previous one's result
            app.rag_state.import_result = None;

            if let Some(ref db) = app.db {
                match db.call::<RagDocumentImport>(&args).await {
                    Ok(result) => {
                        app.rag_state.is_loading = false;
                        app.rag_state.message =
                            Some(crate::modals::rag::import_result_text(&result));
                        app.rag_state.import_result = Some(result);
                    }
                    Err(e) => {
                        app.rag_state.is_loading = false;
                        app.rag_state.error = Some(format!("{}", e));
                    }
                }
            }
        }

        // Toggle mode (when mode field focused)
        (KeyCode::Char(' '), _) if app.rag_state.import_form_field == 4 => {
            app.rag_state.toggle_import_mode();
        }

        // Next form field
        (KeyCode::Tab, KeyModifiers::NONE) => {
            app.rag_state.next_import_field();
        }

        // The content and the title
        _ if app.rag_state.import_form_field <= 1 => {
            app.rag_state.handle_text_input(key, modifiers);
        }

        // Text input based on focused field
        (KeyCode::Char(c), KeyModifiers::NONE) | (KeyCode::Char(c), KeyModifiers::SHIFT) => {
            match app.rag_state.import_form_field {
                2 if c.is_ascii_digit() => {
                    let digit = c.to_digit(10).unwrap() as usize;
                    app.rag_state.import_chunk_size = app.rag_state.import_chunk_size * 10 + digit;
                }
                3 if c.is_ascii_digit() => {
                    let digit = c.to_digit(10).unwrap() as usize;
                    app.rag_state.import_overlap = app.rag_state.import_overlap * 10 + digit;
                }
                _ => {}
            }
            app.rag_state.error = None;
        }
        (KeyCode::Backspace, _) => {
            match app.rag_state.import_form_field {
                2 => {
                    app.rag_state.import_chunk_size /= 10;
                }
                3 => {
                    app.rag_state.import_overlap /= 10;
                }
                _ => {}
            }
            app.rag_state.error = None;
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
    if app.rag_state.creating {
        match (key, modifiers) {
            (KeyCode::Enter, _) => {
                // Create collection
                let name = app
                    .rag_state
                    .create_collection_name
                    .value()
                    .trim()
                    .to_string();
                if name.is_empty() {
                    app.rag_state.error = Some("Collection name is required".to_string());
                    return;
                }

                let args = RagCollectionCreateParams {
                    collection: name,
                    embedding_field: app.rag_state.create_embedding_field.value().to_string(),
                    text_field: app.rag_state.create_text_field.value().to_string(),
                    text_fields: None,
                    context_fields: None,
                    provider: Some(app.rag_state.create_provider.clone()),
                    language: app.rag_state.create_language.clone(),
                };

                app.rag_state.is_loading = true;
                app.rag_state.error = None;

                if let Some(ref db) = app.db {
                    match db.call::<RagCollectionCreate>(&args).await {
                        Ok(response) => {
                            app.rag_state.is_loading = false;
                            app.rag_state.creating = false;
                            app.rag_state.message = Some(rag_created_message(&response));
                        }
                        Err(e) => {
                            app.rag_state.is_loading = false;
                            app.rag_state.error = Some(format!("{}", e));
                        }
                    }
                }
            }

            // Toggle provider/language
            (KeyCode::Char(' '), _) if app.rag_state.create_form_field == 3 => {
                app.rag_state.toggle_create_provider();
            }
            (KeyCode::Char(' '), _) if app.rag_state.create_form_field == 4 => {
                app.rag_state.toggle_create_language();
            }

            // Next field
            (KeyCode::Tab, KeyModifiers::NONE) => {
                app.rag_state.next_create_field();
            }

            // The name and field inputs take the rest
            _ => {
                app.rag_state.handle_text_input(key, modifiers);
            }
        }
        return;
    }

    // Normal mode
    match (key, modifiers) {
        // Create new
        (KeyCode::Char('c'), _) => {
            app.rag_state.creating = true;
            app.rag_state.create_form_field = 0;
            app.rag_state.create_collection_name.clear();
        }

        // Navigate
        (KeyCode::Up, _) | (KeyCode::Char('k'), _) => app.rag_state.collection_up(),
        (KeyCode::Down, _) | (KeyCode::Char('j'), _) => app.rag_state.collection_down(),

        // Stats for selected collection
        (KeyCode::Enter, _) => {
            let Some(name) = app
                .rag_state
                .rag_collections
                .get(app.rag_state.selected_collection)
                .map(|c| c.collection.clone())
            else {
                return;
            };
            if let Some(ref db) = app.db {
                match load_rag_stats(db, &name).await {
                    Ok(stats) => {
                        app.rag_state.message = Some(format!(
                            "{}: {} chunks, {} source docs",
                            stats.collection,
                            stats.stats.chunk_count,
                            stats.stats.source_document_count
                        ));
                        if let Some(entry) = app
                            .rag_state
                            .rag_collections
                            .iter_mut()
                            .find(|c| c.collection == name)
                        {
                            *entry = stats;
                        }
                    }
                    Err(e) => {
                        app.rag_state.error = Some(format!("{}", e));
                    }
                }
            }
        }

        // Refresh
        (KeyCode::Char('r'), _) => {
            let collection = app.rag_state.collection.clone();
            if let Some(ref db) = app.db {
                match load_rag_stats(db, &collection).await {
                    Ok(stats) => {
                        app.rag_state.message = Some("Stats refreshed".to_string());
                        app.rag_state.rag_collections = vec![stats];
                        app.rag_state.selected_collection = 0;
                    }
                    Err(e) => {
                        app.rag_state.error = Some(format!("{}", e));
                    }
                }
            }
        }

        // Tab navigation
        (KeyCode::Tab, KeyModifiers::NONE) => app.rag_state.next_tab(),

        _ => {}
    }
}

/// Models tab key handler
pub(crate) async fn handle_rag_models_key(app: &mut App, key: KeyCode, modifiers: KeyModifiers) {
    // If in test mode
    if app.rag_state.models_mode == 1 {
        match (key, modifiers) {
            (KeyCode::Enter, _) => {
                if app.rag_state.embed_test_input.value().trim().is_empty() {
                    app.rag_state.error = Some("Text is empty".to_string());
                    return;
                }

                let text = app.rag_state.embed_test_input.value().to_string();
                app.rag_state.is_loading = true;
                app.rag_state.error = None;

                if let Some(ref db) = app.db {
                    let args = EmbedTextParams {
                        text,
                        provider: app.rag_state.selected_provider(),
                    };
                    app.rag_state.embed_test_result = None;
                    match db.call::<EmbedText>(&args).await {
                        Ok(result) => {
                            app.rag_state.is_loading = false;
                            app.rag_state.embed_test_result = Some(result);
                        }
                        Err(e) => {
                            app.rag_state.is_loading = false;
                            app.rag_state.error = Some(format!("{}", e));
                        }
                    }
                }
            }

            _ => {
                app.rag_state.handle_text_input(key, modifiers);
            }
        }
        return;
    }

    // Normal mode
    match (key, modifiers) {
        // Navigate models
        (KeyCode::Up, _) | (KeyCode::Char('k'), _) => app.rag_state.model_up(),
        (KeyCode::Down, _) | (KeyCode::Char('j'), _) => app.rag_state.model_down(),

        // Enable auto-embed
        (KeyCode::Char('e'), _) => {
            let collection = app.rag_state.collection.clone();
            if let Some(ref db) = app.db {
                let args = AutoEmbedEnableParams {
                    collection: collection.clone(),
                    source_field: "content".to_string(),
                    target_field: "embedding".to_string(),
                    provider: app.rag_state.selected_provider(),
                    model: None,
                    dimension: None,
                    skip_if_exists: false,
                    chunking: None,
                };
                match db.call::<AutoEmbedEnable>(&args).await {
                    Ok(response) => {
                        // What the server started (backfill / re-embed) and
                        // why the vectors may differ, not a bare "enabled"
                        let mut message = response
                            .message
                            .unwrap_or_else(|| "Auto-embed enabled".to_string());
                        if let Some(job_id) = response.backfill_job_id {
                            message.push_str(&format!(" (job {})", job_id));
                        }
                        if let Some(warning) = response.warning {
                            message.push_str(&format!(" Warning: {}", warning));
                        }
                        app.rag_state.message = Some(message);
                        match load_auto_embed_status(db, &collection).await {
                            Ok(status) => app.rag_state.auto_embed_config = Some(status),
                            Err(e) => {
                                app.rag_state.error = Some(format!("Auto-embed status: {}", e))
                            }
                        }
                    }
                    Err(e) => {
                        app.rag_state.error = Some(format!("{}", e));
                    }
                }
            }
        }

        // Disable auto-embed (with confirmation)
        (KeyCode::Char('d'), _) => app.open_disable_auto_embed_confirm(),

        // Cache stats
        (KeyCode::Char('c'), _) => {
            if let Some(ref db) = app.db {
                match db.call::<EmbedCacheStats>(&NoParams {}).await {
                    Ok(stats) => {
                        app.rag_state.message = Some(match &stats {
                            EmbedCacheStatsResponse::Enabled { .. } => {
                                "Cache stats loaded".to_string()
                            }
                            EmbedCacheStatsResponse::Disabled { message, .. } => message.clone(),
                        });
                        app.rag_state.cache_stats = Some(stats);
                    }
                    Err(e) => {
                        app.rag_state.error = Some(format!("{}", e));
                    }
                }
            }
        }

        // Clear cache (with confirmation)
        (KeyCode::Char('x'), _) => app.open_clear_embed_cache_confirm(),

        // Test embed
        (KeyCode::Char('t'), _) => {
            app.rag_state.models_mode = 1;
            app.rag_state.embed_test_input.clear();
            app.rag_state.embed_test_result = None;
        }

        // Tab navigation
        (KeyCode::Tab, KeyModifiers::NONE) => app.rag_state.next_tab(),

        _ => {}
    }
}

/// Jobs tab key handler
pub(crate) async fn handle_rag_jobs_key(app: &mut App, key: KeyCode) {
    match key {
        // Navigate
        KeyCode::Up | KeyCode::Char('k') => app.rag_state.job_up(),
        KeyCode::Down | KeyCode::Char('j') => app.rag_state.job_down(),

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
                        app.rag_state.set_jobs(result.jobs);
                        app.rag_state.message = Some("Jobs refreshed".to_string());
                    }
                    Err(e) => {
                        app.rag_state.error = Some(format!("{}", e));
                    }
                }
            }
        }

        // Tab navigation
        KeyCode::Tab => app.rag_state.next_tab(),

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
    let created = |yes: bool| if yes { "created" } else { "already existed" };
    format!(
        "RAG collection '{}' ready: vector index {}, {} fulltext index(es) created ({} dims, {})",
        response.collection,
        created(response.indexes.vector_created),
        response.indexes.fulltext_created,
        response.config.dimension,
        response.config.provider
    )
}
