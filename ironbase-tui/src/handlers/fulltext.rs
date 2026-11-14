//! Key handlers: fulltext

use crate::app::{App, FulltextState, Modal};
use crossterm::event::{KeyCode, KeyModifiers};

/// Open fulltext search modal
pub(crate) async fn handle_fulltext_open(app: &mut App) {
    // Need to have a selected collection
    if app.collections.is_empty() {
        app.set_error("Valassz ki egy kollekciot elobb!");
        return;
    }

    let collection = app.current_collection_name().unwrap_or_default();
    if collection.is_empty() {
        app.set_error("Valassz ki egy kollekciot elobb!");
        return;
    }

    // Get list of fulltext indexes for this collection
    let indexed_fields = if let Some(ref db) = app.db {
        match db.list_indexes(collection).await {
            Ok(indexes) => {
                // Filter to only fulltext indexes (format: "{collection}_{field}_fts")
                let prefix = format!("{}_", collection);
                indexes
                    .into_iter()
                    .filter(|idx| idx.ends_with("_fts"))
                    .filter_map(|idx| {
                        // Extract field name: remove "_fts" suffix and collection prefix
                        let without_suffix = idx.trim_end_matches("_fts");
                        without_suffix.strip_prefix(&prefix).map(|s| s.to_string())
                    })
                    .collect()
            }
            Err(_) => Vec::new(),
        }
    } else {
        Vec::new()
    };

    if indexed_fields.is_empty() {
        app.set_error("Nincs fulltext index ezen a kollekcio! Hozz letre egyet elobb.");
        return;
    }

    app.fulltext_state = FulltextState::new(collection.to_string(), indexed_fields);
    app.modal = Some(Modal::Fulltext);
}

/// Async fulltext key handler
pub(crate) async fn handle_fulltext_key_async(
    app: &mut App,
    key: KeyCode,
    modifiers: KeyModifiers,
) {
    match key {
        KeyCode::Esc => app.close_modal(),
        KeyCode::Tab => app.fulltext_state.next_field(),
        // Plain j/k belong to the query input; results: arrows or Ctrl+j/k
        KeyCode::Up => app.fulltext_state.select_up(),
        KeyCode::Down => app.fulltext_state.select_down(),
        KeyCode::Char('k') if modifiers == KeyModifiers::CONTROL => app.fulltext_state.select_up(),
        KeyCode::Char('j') if modifiers == KeyModifiers::CONTROL => {
            app.fulltext_state.select_down()
        }
        KeyCode::Left => app.fulltext_state.move_cursor_left(),
        KeyCode::Right => app.fulltext_state.move_cursor_right(),
        KeyCode::Backspace => app.fulltext_state.backspace(),
        KeyCode::Delete => app.fulltext_state.delete(),
        KeyCode::Enter => {
            // Execute fulltext search
            if app.fulltext_state.query.is_empty() {
                return;
            }

            app.fulltext_state.loading = true;

            if let Some(ref db) = app.db {
                let collection = app.fulltext_state.collection.clone();
                let field = app.fulltext_state.field.clone();
                let query = app.fulltext_state.query.clone();

                match db
                    .fulltext_search(&collection, &field, &query, Some(50))
                    .await
                {
                    Ok(results) => {
                        app.fulltext_state.results = results;
                        app.fulltext_state.selected_result = 0;
                        app.fulltext_state.error = None;
                    }
                    Err(e) => {
                        app.fulltext_state.error = Some(format!("{}", e));
                        app.fulltext_state.results.clear();
                    }
                }
            }

            app.fulltext_state.loading = false;
        }
        KeyCode::Char(c) => {
            app.fulltext_state.insert_char(c);
        }
        _ => {}
    }
}
