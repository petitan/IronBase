//! Listener management tool handlers
//!
//! Uses typed parameter structs for compile-time validation.

use crate::adapter::IronBaseAdapter;
use crate::error::{McpError, Result};
use crate::listener::{ListenerConfig, ListenerManager, SYSTEM_LISTENERS_COLLECTION};
use serde_json::Value;
use std::sync::Arc;

use super::params::{ListenerAddParams, ListenerIdParams, ParseParams};
use ironbase_mcp_types::responses::{
    ListenerAction, ListenerCreateResponse, ListenerDeleteResponse, ListenerDisableResponse,
    ListenerEnableResponse, ListenerInfo, ListenerListResponse,
};

/// A stored listener as the listener tools report it
fn listener_info(config: ListenerConfig) -> ListenerInfo {
    ListenerInfo {
        id: config.id,
        bind: config.bind,
        tls: config.tls,
        cert_path: config.cert_path,
        key_path: config.key_path,
        enabled: config.enabled,
        description: config.description,
    }
}

/// The note of a listener change the server applies on its next start
fn restart_note(found: bool, done: &str) -> String {
    if found {
        format!(
            "Listener {}. Restart server for changes to take effect.",
            done
        )
    } else {
        "Listener not found.".to_string()
    }
}

/// Dispatch listener tool calls
pub fn dispatch(name: &str, params: Value, adapter: &Arc<IronBaseAdapter>) -> Result<Value> {
    match name {
        "listener_list" => handle_listener_list(adapter),
        "listener_get" => handle_listener_get(params, adapter),
        "listener_create" => handle_listener_add(params, adapter),
        "listener_delete" => handle_listener_delete(params, adapter),
        "listener_enable" => handle_listener_enable(params, adapter),
        "listener_disable" => handle_listener_disable(params, adapter),
        _ => Err(McpError::invalid_params(format!(
            "Unknown listener tool: {}",
            name
        ))),
    }
}

fn handle_listener_list(adapter: &Arc<IronBaseAdapter>) -> Result<Value> {
    let manager = ListenerManager::new(adapter.clone());
    let listeners: Vec<ListenerInfo> = manager
        .list()
        .unwrap_or_default()
        .into_iter()
        .map(listener_info)
        .collect();

    Ok(serde_json::to_value(ListenerListResponse {
        count: listeners.len(),
        listeners,
        collection: SYSTEM_LISTENERS_COLLECTION.to_string(),
        note: "Changes require server restart to take effect".to_string(),
    })?)
}

fn handle_listener_get(params: Value, adapter: &Arc<IronBaseAdapter>) -> Result<Value> {
    let p: ListenerIdParams = ListenerIdParams::parse(params)?;
    let manager = ListenerManager::new(adapter.clone());

    match manager.get(&p.id).unwrap_or(None) {
        Some(listener) => Ok(serde_json::to_value(listener_info(listener))?),
        None => Err(McpError::invalid_params(format!(
            "Listener not found: {}",
            p.id
        ))),
    }
}

fn handle_listener_add(params: Value, adapter: &Arc<IronBaseAdapter>) -> Result<Value> {
    let p: ListenerAddParams = ListenerAddParams::parse(params)?;

    let config = ListenerConfig {
        id: p.id.clone(),
        bind: p.bind.clone(),
        tls: p.tls,
        cert_path: p.cert_path,
        key_path: p.key_path,
        enabled: true,
        description: p.description,
    };

    // Validate before saving
    config.validate()?;

    // Atomic upsert - set() returns true if updated, false if created (TOCTOU fix)
    let manager = ListenerManager::new(adapter.clone());
    let is_update = manager.set(&config)?;

    Ok(serde_json::to_value(ListenerCreateResponse {
        success: true,
        id: p.id,
        bind: p.bind,
        tls: p.tls,
        action: if is_update {
            ListenerAction::Updated
        } else {
            ListenerAction::Created
        },
        note: "Restart server for changes to take effect".to_string(),
    })?)
}

fn handle_listener_delete(params: Value, adapter: &Arc<IronBaseAdapter>) -> Result<Value> {
    let p: ListenerIdParams = ListenerIdParams::parse(params)?;

    // Prevent deleting the default listener
    if p.id == "default" {
        return Err(McpError::invalid_params(
            "Cannot delete the default listener. Use listener_disable instead.",
        ));
    }

    let manager = ListenerManager::new(adapter.clone());
    let deleted = manager.delete(&p.id).unwrap_or(false);

    Ok(serde_json::to_value(ListenerDeleteResponse {
        success: deleted,
        id: p.id,
        deleted,
        note: restart_note(deleted, "deleted"),
    })?)
}

fn handle_listener_enable(params: Value, adapter: &Arc<IronBaseAdapter>) -> Result<Value> {
    let p: ListenerIdParams = ListenerIdParams::parse(params)?;
    let manager = ListenerManager::new(adapter.clone());
    let updated = manager.enable(&p.id).unwrap_or(false);

    Ok(serde_json::to_value(ListenerEnableResponse {
        success: updated,
        id: p.id,
        enabled: updated,
        note: restart_note(updated, "enabled"),
    })?)
}

fn handle_listener_disable(params: Value, adapter: &Arc<IronBaseAdapter>) -> Result<Value> {
    let p: ListenerIdParams = ListenerIdParams::parse(params)?;
    let manager = ListenerManager::new(adapter.clone());
    let updated = manager.disable(&p.id).unwrap_or(false);

    Ok(serde_json::to_value(ListenerDisableResponse {
        success: updated,
        id: p.id,
        disabled: updated,
        note: restart_note(updated, "disabled"),
    })?)
}
