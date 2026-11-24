//! Key handlers
//!
//! One module per modal/area. The event loop calls [`handle_key`]; it
//! dispatches to the open modal's handler, or to the global handler when no
//! modal is open.

mod acl;
mod actions;
mod api_key;
mod confirm;
mod database;
mod error;
mod export;
mod filter;
mod fulltext;
mod global;
mod help;
mod index;
mod insert;
mod listener;
mod new_collection;
mod query;
mod rag;
mod script;
mod search;
mod server_info;
mod update;
mod vector;

#[cfg(test)]
mod tests;

use crate::app::{App, Modal, OpenModal};
use crate::widgets::{Input, InputOutcome};
use acl::handle_acl_key_async;
use actions::handle_actions_key_async;
use api_key::handle_api_key_key_async;
use confirm::handle_confirm_key_async;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use database::handle_database_key_async;
use error::handle_error_modal_key;
use export::handle_export_key_async;
use filter::handle_filter_key_async;
use fulltext::handle_fulltext_key_async;
use global::handle_global_key_async;
use help::handle_help_key;
use index::handle_index_key_async;
use insert::handle_insert_key_async;
use listener::handle_listener_key_async;
use new_collection::handle_new_collection_key_async;
use query::handle_query_key_async;
use rag::handle_rag_key_async;
use script::handle_script_key_async;
use search::handle_search_key_async;
use server_info::handle_server_info_key;
use update::handle_update_key;
use vector::handle_vector_key_async;

/// Handle one key press: modal keys first, global keys otherwise
pub async fn handle_key(app: &mut App, key: KeyEvent) {
    if app.modal().is_some() {
        handle_modal_key_async(app, key.code, key.modifiers).await;
    } else {
        handle_global_key_async(app, key.code, key.modifiers).await;
    }
}

/// Handle a pasted text (the terminal's bracketed paste): it goes into the
/// focused input of the open modal, in one piece, through the same entry
/// point as a key (so the same focus rules and side effects apply). Without
/// a focused input, or without a modal, it is dropped: pasted characters are
/// never taken as commands.
pub fn handle_paste(app: &mut App, text: &str) {
    let input = Input::Paste(text);
    match app.modal() {
        Some(Modal::Search) => app.search.handle_input(input),
        Some(Modal::Insert) => app.insert.handle_input(input),
        Some(Modal::Index) => app.index_state.handle_input(input),
        Some(Modal::Query) => app.query_state.handle_input(input),
        Some(Modal::Export) => app.export_state.handle_input(input),
        Some(Modal::Filter) => app.filter_state.handle_field_input(input),
        Some(Modal::NewCollection) => match app.open_modal_mut() {
            Some(OpenModal::NewCollection(state)) => state.handle_input(input),
            _ => InputOutcome::Ignored,
        },
        Some(Modal::Script) => app.script_state.handle_input(input),
        Some(Modal::Database) => app.database_state.handle_input(input),
        Some(Modal::ApiKey) => app.api_key_state.handle_input(input),
        Some(Modal::Fulltext) => app.fulltext_state.handle_input(input),
        Some(Modal::Acl) => app.acl_state.handle_principal_input(input),
        Some(Modal::Listener) => app.listener_state.handle_input(input),
        Some(Modal::VectorSearch) => app.vector_state.handle_text_input(input),
        Some(Modal::Rag) => app.rag_state.handle_text_input(input),
        Some(
            Modal::Actions
            | Modal::Help
            | Modal::Confirm
            | Modal::ErrorDetail
            | Modal::ServerInfo
            | Modal::Update,
        )
        | None => InputOutcome::Ignored,
    };
}

/// Handle modal keys with async support
pub(crate) async fn handle_modal_key_async(app: &mut App, key: KeyCode, modifiers: KeyModifiers) {
    match app.modal() {
        Some(Modal::Search) => handle_search_key_async(app, key, modifiers).await,
        Some(Modal::Actions) => handle_actions_key_async(app, key).await,
        Some(Modal::Help) => {
            handle_help_key(app, key);
        }
        Some(Modal::ErrorDetail) => handle_error_modal_key(app, key),
        Some(Modal::Confirm) => handle_confirm_key_async(app, key).await,
        Some(Modal::Insert) => handle_insert_key_async(app, key, modifiers).await,
        Some(Modal::Index) => handle_index_key_async(app, key, modifiers).await,
        Some(Modal::Query) => handle_query_key_async(app, key, modifiers).await,
        Some(Modal::Export) => handle_export_key_async(app, key, modifiers).await,
        Some(Modal::Filter) => handle_filter_key_async(app, key, modifiers).await,
        Some(Modal::NewCollection) => handle_new_collection_key_async(app, key, modifiers).await,
        Some(Modal::Script) => handle_script_key_async(app, key, modifiers).await,
        Some(Modal::ServerInfo) => handle_server_info_key(app, key),
        Some(Modal::Update) => handle_update_key(app, key),
        Some(Modal::Database) => handle_database_key_async(app, key, modifiers).await,
        Some(Modal::ApiKey) => handle_api_key_key_async(app, key, modifiers).await,
        Some(Modal::Fulltext) => handle_fulltext_key_async(app, key, modifiers).await,
        Some(Modal::Acl) => handle_acl_key_async(app, key, modifiers).await,
        Some(Modal::Listener) => handle_listener_key_async(app, key, modifiers).await,
        Some(Modal::VectorSearch) => handle_vector_key_async(app, key, modifiers).await,
        Some(Modal::Rag) => handle_rag_key_async(app, key, modifiers).await,
        None => {}
    }
}
