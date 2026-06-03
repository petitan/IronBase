//! Listener management tool handlers
//!
//! Uses typed parameter structs for compile-time validation.

use crate::adapter::IronBaseAdapter;
use crate::error::{McpError, Result};
use crate::listener::{ListenerConfig, ListenerManager, SYSTEM_LISTENERS_COLLECTION};
use serde_json::{json, Value};
use std::sync::Arc;

use super::params::{ListenerAddParams, ListenerIdParams, ParseParams};

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
    let listeners = manager.list().unwrap_or_default();

    Ok(json!({
        "listeners": listeners,
        "count": listeners.len(),
        "collection": SYSTEM_LISTENERS_COLLECTION,
        "note": "Changes require server restart to take effect"
    }))
}

fn handle_listener_get(params: Value, adapter: &Arc<IronBaseAdapter>) -> Result<Value> {
    let p: ListenerIdParams = ListenerIdParams::parse(params)?;
    let manager = ListenerManager::new(adapter.clone());

    match manager.get(&p.id).unwrap_or(None) {
        Some(listener) => Ok(serde_json::to_value(listener)?),
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

    Ok(json!({
        "success": true,
        "id": p.id,
        "bind": p.bind,
        "tls": p.tls,
        "action": if is_update { "updated" } else { "created" },
        "note": "Restart server for changes to take effect"
    }))
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

    Ok(json!({
        "success": deleted,
        "id": p.id,
        "deleted": deleted,
        "note": if deleted {
            "Listener deleted. Restart server for changes to take effect."
        } else {
            "Listener not found."
        }
    }))
}

fn handle_listener_enable(params: Value, adapter: &Arc<IronBaseAdapter>) -> Result<Value> {
    let p: ListenerIdParams = ListenerIdParams::parse(params)?;
    let manager = ListenerManager::new(adapter.clone());
    let updated = manager.enable(&p.id).unwrap_or(false);

    Ok(json!({
        "success": updated,
        "id": p.id,
        "enabled": updated,
        "note": if updated {
            "Listener enabled. Restart server for changes to take effect."
        } else {
            "Listener not found."
        }
    }))
}

fn handle_listener_disable(params: Value, adapter: &Arc<IronBaseAdapter>) -> Result<Value> {
    let p: ListenerIdParams = ListenerIdParams::parse(params)?;
    let manager = ListenerManager::new(adapter.clone());
    let updated = manager.disable(&p.id).unwrap_or(false);

    Ok(json!({
        "success": updated,
        "id": p.id,
        "disabled": updated,
        "note": if updated {
            "Listener disabled. Restart server for changes to take effect."
        } else {
            "Listener not found."
        }
    }))
}
