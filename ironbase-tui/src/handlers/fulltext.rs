//! Key handlers: fulltext

use crate::app::{App, FulltextState, OpenModal};
use crate::i18n::{t, Op};
use crate::state::fulltext::FULLTEXT_LIMIT;
use crate::widgets::Input;
use crossterm::event::{KeyCode, KeyModifiers};
use ironbase_mcp_types::params::FulltextSearchParams;
use ironbase_mcp_types::tools::FulltextSearch;

/// The open full-text modal's state, borrowing only `app.open_modal` (so
/// `app.db` stays usable next to it); the handler returns when it is not open
macro_rules! state {
    ($app:expr) => {
        match &mut $app.open_modal {
            Some(OpenModal::Fulltext(state)) => state,
            _ => return,
        }
    };
}

/// Open fulltext search modal
pub(crate) async fn handle_fulltext_open(app: &mut App) {
    // Need to have a selected collection
    if app.collections.is_empty() {
        app.set_error(t().select_collection_first());
        return;
    }

    let collection = app.current_collection_name().unwrap_or_default();
    if collection.is_empty() {
        app.set_error(t().select_collection_first());
        return;
    }

    // The fields of the collection's fulltext indexes, as the server reports them
    let indexed_fields: Vec<String> = if let Some(ref db) = app.db {
        match db.fulltext_indexes(collection).await {
            Ok(indexes) => indexes.into_iter().map(|idx| idx.field).collect(),
            // A failed listing is not "no index": show why it failed
            Err(e) => {
                app.set_error(t().failed(Op::LoadFulltextIndexes, &e));
                return;
            }
        }
    } else {
        Vec::new()
    };

    if indexed_fields.is_empty() {
        app.set_error(t().no_fulltext_index());
        return;
    }

    app.open_modal = Some(OpenModal::Fulltext(FulltextState::new(
        collection.to_string(),
        indexed_fields,
    )));
}

/// Async fulltext key handler
pub(crate) async fn handle_fulltext_key_async(
    app: &mut App,
    key: KeyCode,
    modifiers: KeyModifiers,
) {
    match key {
        KeyCode::Esc => app.close_modal(),
        KeyCode::Tab => state!(app).next_field(),
        // Plain j/k belong to the query input; results: arrows or Ctrl+j/k
        KeyCode::Up => state!(app).select_up(),
        KeyCode::Down => state!(app).select_down(),
        KeyCode::Char('k') if modifiers == KeyModifiers::CONTROL => state!(app).select_up(),
        KeyCode::Char('j') if modifiers == KeyModifiers::CONTROL => state!(app).select_down(),
        KeyCode::Enter => {
            // Execute fulltext search
            if state!(app).query.is_empty() {
                return;
            }

            state!(app).loading = true;

            if let Some(ref db) = app.db {
                let collection = state!(app).collection.clone();
                let field = state!(app).field.clone();
                let query = state!(app).query.value().to_string();

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
                        state!(app).results = response.results;
                        state!(app).selected_result = 0;
                        state!(app).error = None;
                    }
                    Err(e) => {
                        state!(app).error = Some(format!("{}", e));
                        state!(app).results.clear();
                    }
                }
            }

            state!(app).loading = false;
        }
        _ => {
            state!(app).handle_input(Input::Key(key, modifiers));
        }
    }
}
