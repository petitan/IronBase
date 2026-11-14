//! Key handlers: fulltext

use crate::app::{App, FulltextState, Modal};
use crate::state::fulltext::FULLTEXT_LIMIT;
use crossterm::event::{KeyCode, KeyModifiers};
use ironbase_mcp_types::params::FulltextSearchParams;
use ironbase_mcp_types::tools::FulltextSearch;

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

    // The fields of the collection's fulltext indexes, as the server reports them
    let indexed_fields: Vec<String> = if let Some(ref db) = app.db {
        match db.fulltext_indexes(collection).await {
            Ok(indexes) => indexes.into_iter().map(|idx| idx.field).collect(),
            // A failed listing is not "no index": show why it failed
            Err(e) => {
                app.set_error(format!("Fulltext indexek lekerdezese sikertelen: {}", e));
                return;
            }
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

                let args = FulltextSearchParams {
                    collection,
                    field,
                    fields: None,
                    query,
                    limit: Some(FULLTEXT_LIMIT),
                    skip: None,
                    min_score: None,
                    projection: None,
                    mode: None,
                    match_scope: None,
                    filter: None,
                    highlight: false,
                    highlight_context: None,
                    highlight_max_snippets: None,
                };
                match db.call::<FulltextSearch>(&args).await {
                    Ok(response) => {
                        app.fulltext_state.results = response.results;
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
