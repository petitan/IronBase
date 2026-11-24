//! Key handlers: vector

use crate::app::{App, OpenModal, VectorSearchState};
use crate::i18n::t;
use crate::runtime::{Slot, Update};
use crate::state::open_state;
use crate::widgets::Input;
use crossterm::event::{KeyCode, KeyModifiers};
use ironbase_mcp_types::params::{IndexCreateArgs, VectorIndexCreateParams, VectorSearchParams};
use ironbase_mcp_types::tools::{IndexCreate, VectorSearch};

/// Open vector search modal
pub(crate) async fn handle_vector_open(app: &mut App) {
    // Need to have a selected collection
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

    // Initialize vector state
    app.open_modal = Some(OpenModal::Vector(VectorSearchState::new(
        collection.clone(),
    )));

    // Load vector indexes for this collection
    if let Some(ref db) = app.db {
        if let Ok(indexes) = db.vector_indexes(&collection).await {
            open_state!(app, Vector).update_indexes(indexes);

            // Pre-fill search field if we have an index
            if let Some(first_idx) = open_state!(app, Vector).indexes.first() {
                open_state!(app, Vector).search_field = first_idx.field.clone();
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
        (KeyCode::Tab, KeyModifiers::NONE) => open_state!(app, Vector).next_tab(),
        (KeyCode::BackTab, _) => open_state!(app, Vector).prev_tab(),

        _ => {
            // Tab-specific handling
            match open_state!(app, Vector).active_tab {
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
            let vector = match open_state!(app, Vector).parse_vector() {
                Ok(v) => v,
                Err(e) => {
                    open_state!(app, Vector).error = Some(e);
                    return;
                }
            };

            // Parse optional filter
            let filter = match open_state!(app, Vector).parse_filter() {
                Ok(f) => f,
                Err(e) => {
                    open_state!(app, Vector).error = Some(e);
                    return;
                }
            };

            let collection = open_state!(app, Vector).collection.clone();
            let field = open_state!(app, Vector).search_field.clone();
            let k = open_state!(app, Vector).k;

            if field.is_empty() {
                open_state!(app, Vector).error = Some(t().field_name_required().to_string());
                return;
            }

            let Some(db) = app.db.clone() else {
                return;
            };
            open_state!(app, Vector).is_loading = true;
            open_state!(app, Vector).error = None;

            let args = VectorSearchParams {
                collection,
                field,
                vector,
                limit: k,
                projection: None,
            };
            // In the background: the screen keeps drawing
            app.tasks.spawn(Slot::Modal, async move {
                let result = if let Some(f) = filter {
                    db.vector_search_filter(&args, &f).await
                } else {
                    db.call::<VectorSearch>(&args).await.map(|r| r.results)
                };
                Box::new(move |app: &mut App| {
                    let Some(state) = app.vector_state_mut() else {
                        return;
                    };
                    match result {
                        Ok(results) => {
                            state.update_results(results);
                            state.message = Some(t().match_count(state.results.len()));
                        }
                        Err(e) => {
                            state.is_loading = false;
                            state.error = Some(format!("{}", e));
                        }
                    }
                }) as Update
            });
        }

        // Navigation in results
        (KeyCode::Up, _) | (KeyCode::Char('k'), _) => open_state!(app, Vector).result_up(),
        (KeyCode::Down, _) | (KeyCode::Char('j'), _) => open_state!(app, Vector).result_down(),

        // Text input for vector
        _ => {
            open_state!(app, Vector).handle_text_input(Input::Key(key, modifiers));
        }
    }
}

/// Handle keys in Create Index tab
pub(crate) async fn handle_vector_create_key(app: &mut App, key: KeyCode, modifiers: KeyModifiers) {
    match (key, modifiers) {
        // Create index
        (KeyCode::Enter, _) => {
            let collection = open_state!(app, Vector).collection.clone();
            let field = open_state!(app, Vector)
                .new_field
                .value()
                .trim()
                .to_string();
            let dimension = open_state!(app, Vector).dimension;
            let metric = open_state!(app, Vector).metric.as_str().to_string();

            if field.is_empty() {
                open_state!(app, Vector).error = Some(t().field_name_required().to_string());
                return;
            }

            if dimension == 0 {
                open_state!(app, Vector).error = Some(t().dimension_positive().to_string());
                return;
            }

            open_state!(app, Vector).is_loading = true;
            open_state!(app, Vector).error = None;

            if let Some(ref db) = app.db {
                let args = IndexCreateArgs::Vector(VectorIndexCreateParams::new(
                    collection.clone(),
                    field,
                    dimension,
                    metric,
                ));
                match db.call::<IndexCreate>(&args).await {
                    Ok(created) => {
                        // After clearing the form, which also clears the message
                        open_state!(app, Vector).clear_create_form();
                        open_state!(app, Vector).message =
                            Some(t().vector_index_created(created.index_name()));

                        // Refresh index list
                        if let Ok(indexes) = db.vector_indexes(&collection).await {
                            open_state!(app, Vector).update_indexes(indexes);
                        }
                    }
                    Err(e) => {
                        open_state!(app, Vector).is_loading = false;
                        open_state!(app, Vector).error = Some(format!("{}", e));
                    }
                }
            }
        }

        // Next form field
        (KeyCode::Tab, KeyModifiers::NONE) => open_state!(app, Vector).next_create_field(),

        // Toggle metric
        (KeyCode::Char(' '), _) if open_state!(app, Vector).create_form_field == 2 => {
            open_state!(app, Vector).toggle_metric();
        }

        // Field name input
        _ if open_state!(app, Vector).create_form_field == 0 => {
            open_state!(app, Vector).handle_text_input(Input::Key(key, modifiers));
        }

        // Dimension (digits only)
        (KeyCode::Char(c), KeyModifiers::NONE) | (KeyCode::Char(c), KeyModifiers::SHIFT)
            if open_state!(app, Vector).create_form_field == 1 =>
        {
            open_state!(app, Vector).dimension =
                crate::widgets::push_digits(open_state!(app, Vector).dimension, &c.to_string());
            open_state!(app, Vector).error = None;
        }
        (KeyCode::Backspace, _) if open_state!(app, Vector).create_form_field == 1 => {
            open_state!(app, Vector).dimension /= 10;
            open_state!(app, Vector).error = None;
        }

        _ => {}
    }
}

/// Handle keys in List Indexes tab
pub(crate) async fn handle_vector_list_key(app: &mut App, key: KeyCode) {
    match key {
        // Navigation
        KeyCode::Up | KeyCode::Char('k') => open_state!(app, Vector).index_up(),
        KeyCode::Down | KeyCode::Char('j') => open_state!(app, Vector).index_down(),

        // Delete index (with confirmation)
        KeyCode::Char('d') | KeyCode::Delete => app.open_drop_vector_index_confirm(),

        _ => {}
    }
}
