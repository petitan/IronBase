//! Listener management tool handlers

use crate::adapter::IronBaseAdapter;
use crate::error::{McpError, Result};
use crate::listener::{ListenerConfig, ListenerManager, SYSTEM_LISTENERS_COLLECTION};
use serde_json::{json, Value};
use std::sync::Arc;

use super::helpers::get_string;

/// Dispatch listener tool calls
pub fn dispatch(name: &str, params: Value, adapter: &Arc<IronBaseAdapter>) -> Result<Value> {
    match name {
        "listener_list" => handle_listener_list(adapter),
        "listener_get" => handle_listener_get(params, adapter),
        "listener_add" => handle_listener_add(params, adapter),
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
    let id = get_string(&params, "id")?;
    let manager = ListenerManager::new(adapter.clone());

    match manager.get(&id).unwrap_or(None) {
        Some(listener) => Ok(serde_json::to_value(listener)?),
        None => Err(McpError::invalid_params(format!(
            "Listener not found: {}",
            id
        ))),
    }
}

fn handle_listener_add(params: Value, adapter: &Arc<IronBaseAdapter>) -> Result<Value> {
    let id = get_string(&params, "id")?;
    let bind = get_string(&params, "bind")?;
    let tls = params.get("tls").and_then(|v| v.as_bool()).unwrap_or(false);
    let cert_path = params
        .get("cert_path")
        .and_then(|v| v.as_str())
        .map(String::from);
    let key_path = params
        .get("key_path")
        .and_then(|v| v.as_str())
        .map(String::from);
    let description = params
        .get("description")
        .and_then(|v| v.as_str())
        .map(String::from);

    let config = ListenerConfig {
        id: id.clone(),
        bind: bind.clone(),
        tls,
        cert_path,
        key_path,
        enabled: true,
        description,
    };

    // Validate before saving
    config.validate()?;

    let manager = ListenerManager::new(adapter.clone());
    let is_update = manager.get(&id).unwrap_or(None).is_some();
    manager.set(&config)?;

    Ok(json!({
        "success": true,
        "id": id,
        "bind": bind,
        "tls": tls,
        "action": if is_update { "updated" } else { "created" },
        "note": "Restart server for changes to take effect"
    }))
}

fn handle_listener_delete(params: Value, adapter: &Arc<IronBaseAdapter>) -> Result<Value> {
    let id = get_string(&params, "id")?;

    // Prevent deleting the default listener
    if id == "default" {
        return Err(McpError::invalid_params(
            "Cannot delete the default listener. Use listener_disable instead.",
        ));
    }

    let manager = ListenerManager::new(adapter.clone());
    let deleted = manager.delete(&id).unwrap_or(false);

    Ok(json!({
        "success": true,
        "id": id,
        "deleted": deleted,
        "note": if deleted {
            "Listener deleted. Restart server for changes to take effect."
        } else {
            "Listener not found."
        }
    }))
}

fn handle_listener_enable(params: Value, adapter: &Arc<IronBaseAdapter>) -> Result<Value> {
    let id = get_string(&params, "id")?;
    let manager = ListenerManager::new(adapter.clone());
    let updated = manager.enable(&id).unwrap_or(false);

    Ok(json!({
        "success": true,
        "id": id,
        "enabled": updated,
        "note": if updated {
            "Listener enabled. Restart server for changes to take effect."
        } else {
            "Listener not found."
        }
    }))
}

fn handle_listener_disable(params: Value, adapter: &Arc<IronBaseAdapter>) -> Result<Value> {
    let id = get_string(&params, "id")?;
    let manager = ListenerManager::new(adapter.clone());
    let updated = manager.disable(&id).unwrap_or(false);

    Ok(json!({
        "success": true,
        "id": id,
        "disabled": updated,
        "note": if updated {
            "Listener disabled. Restart server for changes to take effect."
        } else {
            "Listener not found."
        }
    }))
}
