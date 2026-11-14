//! Key handlers: vector

use crate::app::{App, Modal, VectorSearchState};
use crossterm::event::{KeyCode, KeyModifiers};

/// Open vector search modal
pub(crate) async fn handle_vector_open(app: &mut App) {
    // Need to have a selected collection
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

    // Initialize vector state
    app.vector_state = VectorSearchState::new(collection.clone());
    app.modal = Some(Modal::VectorSearch);

    // Load vector indexes for this collection
    if let Some(ref db) = app.db {
        if let Ok(indexes) = db.list_vector_indexes(&collection).await {
            app.vector_state.update_indexes(indexes);

            // Pre-fill search field if we have an index
            if let Some(first_idx) = app.vector_state.indexes.first() {
                app.vector_state.search_field = first_idx.field.clone();
            }
        }
    }
}

/// Handle vector modal keys
pub(crate) async fn handle_vector_key_async(app: &mut App, key: KeyCode, modifiers: KeyModifiers) {
    use crate::state::vector::VectorTab;

    match (key, modifiers) {
        // Close modal
        (KeyCode::Esc, _) => app.close_modal(),

        // Tab navigation
        (KeyCode::Tab, KeyModifiers::NONE) => app.vector_state.next_tab(),
        (KeyCode::BackTab, _) => app.vector_state.prev_tab(),

        _ => {
            // Tab-specific handling
            match app.vector_state.active_tab {
                VectorTab::Search => handle_vector_search_key(app, key, modifiers).await,
                VectorTab::CreateIndex => handle_vector_create_key(app, key, modifiers).await,
                VectorTab::ListIndexes => handle_vector_list_key(app, key).await,
            }
        }
    }
}

/// Handle keys in Search tab
pub(crate) async fn handle_vector_search_key(app: &mut App, key: KeyCode, modifiers: KeyModifiers) {
    match (key, modifiers) {
        // Execute search
        (KeyCode::Enter, _) => {
            // Parse vector
            let vector = match app.vector_state.parse_vector() {
                Ok(v) => v,
                Err(e) => {
                    app.vector_state.error = Some(e);
                    return;
                }
            };

            // Parse optional filter
            let filter = match app.vector_state.parse_filter() {
                Ok(f) => f,
                Err(e) => {
                    app.vector_state.error = Some(e);
                    return;
                }
            };

            let collection = app.vector_state.collection.clone();
            let field = app.vector_state.search_field.clone();
            let k = app.vector_state.k;

            if field.is_empty() {
                app.vector_state.error = Some("Field name is required".to_string());
                return;
            }

            app.vector_state.is_loading = true;
            app.vector_state.error = None;

            if let Some(ref db) = app.db {
                let result = if let Some(f) = filter {
                    db.vector_search_filter(&collection, &field, &vector, &f, k)
                        .await
                } else {
                    db.vector_search(&collection, &field, &vector, k).await
                };

                match result {
                    Ok(results) => {
                        app.vector_state.update_results(results);
                        app.vector_state.message =
                            Some(format!("Found {} results", app.vector_state.results.len()));
                    }
                    Err(e) => {
                        app.vector_state.is_loading = false;
                        app.vector_state.error = Some(format!("{}", e));
                    }
                }
            }
        }

        // Navigation in results
        (KeyCode::Up, _) | (KeyCode::Char('k'), _) => app.vector_state.result_up(),
        (KeyCode::Down, _) | (KeyCode::Char('j'), _) => app.vector_state.result_down(),

        // Text input for vector
        (KeyCode::Char(c), KeyModifiers::NONE) | (KeyCode::Char(c), KeyModifiers::SHIFT) => {
            app.vector_state.vector_input.push(c);
            app.vector_state.error = None;
        }
        (KeyCode::Backspace, _) => {
            app.vector_state.vector_input.pop();
            app.vector_state.error = None;
        }

        _ => {}
    }
}

/// Handle keys in Create Index tab
pub(crate) async fn handle_vector_create_key(app: &mut App, key: KeyCode, modifiers: KeyModifiers) {
    match (key, modifiers) {
        // Create index
        (KeyCode::Enter, _) => {
            let collection = app.vector_state.collection.clone();
            let field = app.vector_state.new_field.trim().to_string();
            let dimension = app.vector_state.dimension;
            let metric = app.vector_state.metric.as_str().to_string();

            if field.is_empty() {
                app.vector_state.error = Some("Field name is required".to_string());
                return;
            }

            if dimension == 0 {
                app.vector_state.error = Some("Dimension must be > 0".to_string());
                return;
            }

            app.vector_state.is_loading = true;
            app.vector_state.error = None;

            if let Some(ref db) = app.db {
                match db
                    .create_vector_index(&collection, &field, dimension, &metric)
                    .await
                {
                    Ok(name) => {
                        app.vector_state.message = Some(format!("Index '{}' created!", name));
                        app.vector_state.clear_create_form();

                        // Refresh index list
                        if let Ok(indexes) = db.list_vector_indexes(&collection).await {
                            app.vector_state.update_indexes(indexes);
                        }
                    }
                    Err(e) => {
                        app.vector_state.is_loading = false;
                        app.vector_state.error = Some(format!("{}", e));
                    }
                }
            }
        }

        // Next form field
        (KeyCode::Tab, KeyModifiers::NONE) => app.vector_state.next_create_field(),

        // Toggle metric
        (KeyCode::Char(' '), _) if app.vector_state.create_form_field == 2 => {
            app.vector_state.toggle_metric();
        }

        // Text input
        (KeyCode::Char(c), KeyModifiers::NONE) | (KeyCode::Char(c), KeyModifiers::SHIFT) => {
            match app.vector_state.create_form_field {
                0 => app.vector_state.new_field.push(c),
                1 if c.is_ascii_digit() => {
                    let digit = c.to_digit(10).unwrap() as usize;
                    app.vector_state.dimension = app.vector_state.dimension * 10 + digit;
                }
                _ => {}
            }
            app.vector_state.error = None;
        }
        (KeyCode::Backspace, _) => {
            match app.vector_state.create_form_field {
                0 => {
                    app.vector_state.new_field.pop();
                }
                1 => {
                    app.vector_state.dimension /= 10;
                }
                _ => {}
            }
            app.vector_state.error = None;
        }

        _ => {}
    }
}

/// Handle keys in List Indexes tab
pub(crate) async fn handle_vector_list_key(app: &mut App, key: KeyCode) {
    match key {
        // Navigation
        KeyCode::Up | KeyCode::Char('k') => app.vector_state.index_up(),
        KeyCode::Down | KeyCode::Char('j') => app.vector_state.index_down(),

        // Delete index (with confirmation)
        KeyCode::Char('d') | KeyCode::Delete => app.open_drop_vector_index_confirm(),

        _ => {}
    }
}
