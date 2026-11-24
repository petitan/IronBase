//! Key handlers: fulltext

use crate::app::{App, FulltextState, OpenModal};
use crate::i18n::{t, Op};
use crate::state::fulltext::FULLTEXT_LIMIT;
use crate::state::open_state;
use crate::widgets::Input;
use crossterm::event::{KeyCode, KeyModifiers};
use ironbase_mcp_types::params::FulltextSearchParams;
use ironbase_mcp_types::tools::FulltextSearch;

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
        KeyCode::Tab => open_state!(app, Fulltext).next_field(),
        // Plain j/k belong to the query input; results: arrows or Ctrl+j/k
        KeyCode::Up => open_state!(app, Fulltext).select_up(),
        KeyCode::Down => open_state!(app, Fulltext).select_down(),
        KeyCode::Char('k') if modifiers == KeyModifiers::CONTROL => {
            open_state!(app, Fulltext).select_up()
        }
        KeyCode::Char('j') if modifiers == KeyModifiers::CONTROL => {
            open_state!(app, Fulltext).select_down()
        }
        KeyCode::Enter => {
            // Execute fulltext search
            if open_state!(app, Fulltext).query.is_empty() {
                return;
            }

            open_state!(app, Fulltext).loading = true;

            if let Some(ref db) = app.db {
                let collection = open_state!(app, Fulltext).collection.clone();
                let field = open_state!(app, Fulltext).field.clone();
                let query = open_state!(app, Fulltext).query.value().to_string();

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
                        open_state!(app, Fulltext).results = response.results;
                        open_state!(app, Fulltext).selected_result = 0;
                        open_state!(app, Fulltext).error = None;
                    }
                    Err(e) => {
                        open_state!(app, Fulltext).error = Some(format!("{}", e));
                        open_state!(app, Fulltext).results.clear();
                    }
                }
            }

            open_state!(app, Fulltext).loading = false;
        }
        _ => {
            open_state!(app, Fulltext).handle_input(Input::Key(key, modifiers));
        }
    }
}
