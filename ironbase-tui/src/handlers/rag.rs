//! Key handlers: rag

use crate::app::{App, Modal, RagState};
use crossterm::event::{KeyCode, KeyModifiers};

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
        if let Ok(models_result) = db.embed_list_models().await {
            if let Some(models) = models_result.get("models").and_then(|v| v.as_array()) {
                app.rag_state.models = models.clone();
            }
        }
        if let Ok(ae_status) = db.auto_embed_status(&collection).await {
            app.rag_state.auto_embed_config = Some(ae_status);
        }
    }
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
            if app.rag_state.search_query.trim().is_empty() {
                app.rag_state.error = Some("Query is empty".to_string());
                return;
            }

            let collection = app.rag_state.collection.clone();
            let query = app.rag_state.search_query.clone();
            let limit = app.rag_state.search_limit;
            let mode = app.rag_state.search_mode.clone();
            let rrf_k = app.rag_state.search_rrf_k;

            app.rag_state.is_loading = true;
            app.rag_state.error = None;

            if let Some(ref db) = app.db {
                match db
                    .hybrid_search_rag(&collection, &query, limit, &mode, rrf_k)
                    .await
                {
                    Ok(result) => {
                        let results: Vec<serde_json::Value> = result
                            .get("results")
                            .and_then(|v| serde_json::from_value(v.clone()).ok())
                            .unwrap_or_default();
                        let count = results.len();
                        app.rag_state.search_results = results;
                        app.rag_state.selected_result = 0;
                        app.rag_state.is_loading = false;
                        app.rag_state.message = Some(format!("Found {} results", count));
                    }
                    Err(e) => {
                        app.rag_state.is_loading = false;
                        app.rag_state.error = Some(format!("{}", e));
                    }
                }
            }
        }

        // Switch focus between query and options
        (KeyCode::Tab, KeyModifiers::NONE) => {
            if app.rag_state.search_focus == 0 {
                app.rag_state.search_focus = 1;
                app.rag_state.search_option_idx = 0;
            } else {
                // Cycle through options, then back to query
                if app.rag_state.search_option_idx < 2 {
                    app.rag_state.search_option_idx += 1;
                } else {
                    app.rag_state.search_focus = 0;
                }
            }
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

        // Adjust option values with Up/Down when focus is on options
        (KeyCode::Up, _) if app.rag_state.search_focus == 1 => {
            match app.rag_state.search_option_idx {
                0 if app.rag_state.search_limit < 100 => {
                    app.rag_state.search_limit += 1;
                }
                1 => {
                    app.rag_state.toggle_search_mode();
                }
                2 => {
                    app.rag_state.search_rrf_k += 5.0;
                }
                _ => {}
            }
        }
        (KeyCode::Down, _) if app.rag_state.search_focus == 1 => {
            match app.rag_state.search_option_idx {
                0 if app.rag_state.search_limit > 1 => {
                    app.rag_state.search_limit -= 1;
                }
                1 => {
                    app.rag_state.toggle_search_mode();
                }
                2 if app.rag_state.search_rrf_k > 5.0 => {
                    app.rag_state.search_rrf_k -= 5.0;
                }
                _ => {}
            }
        }

        // Text input for query (when query focused)
        (KeyCode::Char(c), KeyModifiers::NONE) | (KeyCode::Char(c), KeyModifiers::SHIFT)
            if app.rag_state.search_focus == 0 =>
        {
            app.rag_state.search_query.push(c);
            app.rag_state.error = None;
        }
        (KeyCode::Backspace, _) if app.rag_state.search_focus == 0 => {
            app.rag_state.search_query.pop();
            app.rag_state.error = None;
        }

        _ => {}
    }
}

/// Import tab key handler
pub(crate) async fn handle_rag_import_key(app: &mut App, key: KeyCode, modifiers: KeyModifiers) {
    match (key, modifiers) {
        // Execute import
        (KeyCode::Enter, _) => {
            if app.rag_state.import_content.trim().is_empty() {
                app.rag_state.error = Some("Content is empty".to_string());
                return;
            }

            let collection = app.rag_state.collection.clone();
            let content = app.rag_state.import_content.clone();
            let title = if app.rag_state.import_title.is_empty() {
                None
            } else {
                Some(app.rag_state.import_title.as_str())
            };
            let chunk_size = app.rag_state.import_chunk_size;
            let overlap = app.rag_state.import_overlap;
            let mode = app.rag_state.import_mode.clone();

            app.rag_state.is_loading = true;
            app.rag_state.error = None;

            if let Some(ref db) = app.db {
                match db
                    .rag_document_import(&collection, &content, title, chunk_size, overlap, &mode)
                    .await
                {
                    Ok(result) => {
                        app.rag_state.is_loading = false;
                        let chunks_created = result
                            .get("chunks_created")
                            .and_then(|v| v.as_u64())
                            .unwrap_or(0);
                        app.rag_state.import_result = Some(result);
                        app.rag_state.message =
                            Some(format!("Import OK: {} chunks", chunks_created));
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

        // Text input based on focused field
        (KeyCode::Char(c), KeyModifiers::NONE) | (KeyCode::Char(c), KeyModifiers::SHIFT) => {
            match app.rag_state.import_form_field {
                0 => app.rag_state.import_content.push(c),
                1 => app.rag_state.import_title.push(c),
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
                0 => {
                    app.rag_state.import_content.pop();
                }
                1 => {
                    app.rag_state.import_title.pop();
                }
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
                let name = app.rag_state.create_collection_name.trim().to_string();
                if name.is_empty() {
                    app.rag_state.error = Some("Collection name is required".to_string());
                    return;
                }

                let emb_field = app.rag_state.create_embedding_field.clone();
                let text_field = app.rag_state.create_text_field.clone();
                let provider = app.rag_state.create_provider.clone();
                let language = app.rag_state.create_language.clone();

                app.rag_state.is_loading = true;
                app.rag_state.error = None;

                if let Some(ref db) = app.db {
                    match db
                        .rag_collection_create(&name, &emb_field, &text_field, &provider, &language)
                        .await
                    {
                        Ok(_) => {
                            app.rag_state.is_loading = false;
                            app.rag_state.creating = false;
                            app.rag_state.message = Some(format!("Collection '{}' created!", name));
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

            // Text input
            (KeyCode::Char(c), KeyModifiers::NONE) | (KeyCode::Char(c), KeyModifiers::SHIFT) => {
                match app.rag_state.create_form_field {
                    0 => app.rag_state.create_collection_name.push(c),
                    1 => app.rag_state.create_embedding_field.push(c),
                    2 => app.rag_state.create_text_field.push(c),
                    _ => {}
                }
                app.rag_state.error = None;
            }
            (KeyCode::Backspace, _) => {
                match app.rag_state.create_form_field {
                    0 => {
                        app.rag_state.create_collection_name.pop();
                    }
                    1 => {
                        app.rag_state.create_embedding_field.pop();
                    }
                    2 => {
                        app.rag_state.create_text_field.pop();
                    }
                    _ => {}
                }
                app.rag_state.error = None;
            }

            _ => {}
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
            if let Some(coll) = app
                .rag_state
                .rag_collections
                .get(app.rag_state.selected_collection)
            {
                let name = coll
                    .get("collection")
                    .or_else(|| coll.get("name"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                if !name.is_empty() {
                    if let Some(ref db) = app.db {
                        match db.rag_collection_stats(&name).await {
                            Ok(stats) => {
                                let chunk_count = stats
                                    .get("chunk_count")
                                    .and_then(|v| v.as_u64())
                                    .unwrap_or(0);
                                let source_docs = stats
                                    .get("source_doc_count")
                                    .and_then(|v| v.as_u64())
                                    .unwrap_or(0);
                                app.rag_state.message = Some(format!(
                                    "{}: {} chunks, {} source docs",
                                    name, chunk_count, source_docs
                                ));
                            }
                            Err(e) => {
                                app.rag_state.error = Some(format!("{}", e));
                            }
                        }
                    }
                }
            }
        }

        // Refresh
        (KeyCode::Char('r'), _) => {
            let collection = app.rag_state.collection.clone();
            if let Some(ref db) = app.db {
                match db.rag_collection_stats(&collection).await {
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
                if app.rag_state.embed_test_input.trim().is_empty() {
                    app.rag_state.error = Some("Text is empty".to_string());
                    return;
                }

                let text = app.rag_state.embed_test_input.clone();
                app.rag_state.is_loading = true;
                app.rag_state.error = None;

                if let Some(ref db) = app.db {
                    match db.embed_text(&text, "ollama").await {
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

            (KeyCode::Char(c), KeyModifiers::NONE) | (KeyCode::Char(c), KeyModifiers::SHIFT) => {
                app.rag_state.embed_test_input.push(c);
                app.rag_state.error = None;
            }
            (KeyCode::Backspace, _) => {
                app.rag_state.embed_test_input.pop();
                app.rag_state.error = None;
            }

            _ => {}
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
                match db
                    .auto_embed_enable(&collection, "content", "embedding", "ollama")
                    .await
                {
                    Ok(_) => {
                        app.rag_state.message = Some("Auto-embed enabled!".to_string());
                        if let Ok(status) = db.auto_embed_status(&collection).await {
                            app.rag_state.auto_embed_config = Some(status);
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
                match db.embed_cache_stats().await {
                    Ok(stats) => {
                        app.rag_state.cache_stats = Some(stats);
                        app.rag_state.message = Some("Cache stats loaded".to_string());
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
                match db.embed_job_list(false).await {
                    Ok(result) => {
                        if let Some(jobs) = result.get("jobs").and_then(|v| v.as_array()) {
                            app.rag_state.jobs = jobs.clone();
                        }
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
