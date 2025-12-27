//! Admin tool handlers (require IRONBASE_ADMIN_KEY)

use crate::adapter::IronBaseAdapter;
use crate::api_keys::ApiKeyCache;
use crate::error::{McpError, Result};
use crate::ServerInfo;
use serde_json::{json, Value};
use std::sync::Arc;

use super::helpers::{get_string, verify_admin_key};

/// Dispatch admin tool calls
pub fn dispatch(
    name: &str,
    params: Value,
    adapter: &Arc<IronBaseAdapter>,
    api_key_cache: Option<&ApiKeyCache>,
    server_info: Option<&ServerInfo>,
) -> Result<Value> {
    match name {
        "db_open" => handle_db_open(params, adapter),
        "db_stats" => handle_db_stats(adapter, server_info),
        "db_compact" => handle_db_compact(adapter),
        "db_checkpoint" => handle_db_checkpoint(adapter),
        "admin_list_all_collections" => handle_admin_list_all_collections(params, adapter),
        "admin_create_system_collection" => {
            handle_admin_create_system_collection(params, adapter)
        }
        "admin_set_collection_flags" => handle_admin_set_collection_flags(params, adapter),
        "admin_drop_protected" => handle_admin_drop_protected(params, adapter),
        "admin_apikey_create" => handle_admin_apikey_create(params, adapter, api_key_cache),
        "admin_apikey_list" => handle_admin_apikey_list(params, adapter),
        "admin_apikey_revoke" => handle_admin_apikey_revoke(params, adapter, api_key_cache),
        "admin_apikey_delete" => handle_admin_apikey_delete(params, adapter, api_key_cache),
        _ => Err(McpError::InvalidParams(format!(
            "Unknown admin tool: {}",
            name
        ))),
    }
}

fn handle_db_open(params: Value, adapter: &Arc<IronBaseAdapter>) -> Result<Value> {
    let path = get_string(&params, "path")?;
    let create = params
        .get("create")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let new_path = adapter.switch_database(&path, create)?;

    Ok(json!({
        "success": true,
        "path": new_path,
        "message": if create { "Database created" } else { "Database opened" }
    }))
}

fn handle_db_stats(adapter: &Arc<IronBaseAdapter>, server_info: Option<&ServerInfo>) -> Result<Value> {
    let mut stats = adapter.stats();
    // Add server info if available
    if let Some(info) = server_info {
        if let Some(obj) = stats.as_object_mut() {
            obj.insert(
                "server".to_string(),
                json!({
                    "version": crate::VERSION,
                    "protocol": info.protocol,
                    "host": info.host,
                    "port": info.port,
                    "require_api_key": info.require_api_key,
                }),
            );
        }
    } else {
        // Stdio mode - no server info
        if let Some(obj) = stats.as_object_mut() {
            obj.insert(
                "server".to_string(),
                json!({
                    "version": crate::VERSION,
                    "mode": "stdio",
                }),
            );
        }
    }
    Ok(stats)
}

fn handle_db_compact(adapter: &Arc<IronBaseAdapter>) -> Result<Value> {
    adapter.compact()
}

fn handle_db_checkpoint(adapter: &Arc<IronBaseAdapter>) -> Result<Value> {
    adapter.checkpoint()?;
    Ok(json!({"success": true, "message": "Checkpoint completed"}))
}

fn handle_admin_list_all_collections(
    params: Value,
    adapter: &Arc<IronBaseAdapter>,
) -> Result<Value> {
    verify_admin_key(&params)?;
    let collections = adapter.list_all_collections();
    Ok(json!({"collections": collections, "count": collections.len()}))
}

fn handle_admin_create_system_collection(
    params: Value,
    adapter: &Arc<IronBaseAdapter>,
) -> Result<Value> {
    verify_admin_key(&params)?;
    let name = get_string(&params, "name")?;
    adapter.create_system_collection(&name)?;
    Ok(
        json!({"success": true, "collection": name, "flags": {"is_system": true, "protected": true, "hidden": false}}),
    )
}

fn handle_admin_set_collection_flags(
    params: Value,
    adapter: &Arc<IronBaseAdapter>,
) -> Result<Value> {
    verify_admin_key(&params)?;
    let collection = get_string(&params, "collection")?;
    let is_system = params.get("is_system").and_then(|v| v.as_bool());
    let protected = params.get("protected").and_then(|v| v.as_bool());
    let hidden = params.get("hidden").and_then(|v| v.as_bool());
    adapter.set_collection_flags(&collection, is_system, protected, hidden)?;
    Ok(
        json!({"success": true, "collection": collection, "flags": {"is_system": is_system, "protected": protected, "hidden": hidden}}),
    )
}

fn handle_admin_drop_protected(params: Value, adapter: &Arc<IronBaseAdapter>) -> Result<Value> {
    verify_admin_key(&params)?;
    let name = get_string(&params, "name")?;
    adapter.force_drop_collection(&name)?;
    Ok(json!({"success": true, "dropped": name}))
}

fn handle_admin_apikey_create(
    params: Value,
    adapter: &Arc<IronBaseAdapter>,
    api_key_cache: Option<&ApiKeyCache>,
) -> Result<Value> {
    verify_admin_key(&params)?;
    let name = get_string(&params, "name")?;

    // Use provided cache or create temporary one (for stdio mode)
    let temp_cache;
    let cache = match api_key_cache {
        Some(c) => c,
        None => {
            temp_cache = ApiKeyCache::new(60, false);
            &temp_cache
        }
    };

    match crate::api_keys::create_api_key(adapter, &name, cache) {
        Ok(api_key) => Ok(json!({
            "success": true,
            "id": api_key._id,
            "key": api_key.key,
            "name": api_key.name,
            "created_at": api_key.created_at,
            "note": "Save this key now - it cannot be retrieved later!"
        })),
        Err(e) => Err(McpError::Internal(e)),
    }
}

fn handle_admin_apikey_list(params: Value, adapter: &Arc<IronBaseAdapter>) -> Result<Value> {
    verify_admin_key(&params)?;
    match crate::api_keys::list_api_keys(adapter) {
        Ok(keys) => Ok(json!({
            "success": true,
            "keys": keys,
            "count": keys.len()
        })),
        Err(e) => Err(McpError::Internal(e)),
    }
}

fn handle_admin_apikey_revoke(
    params: Value,
    adapter: &Arc<IronBaseAdapter>,
    api_key_cache: Option<&ApiKeyCache>,
) -> Result<Value> {
    verify_admin_key(&params)?;
    let id = params
        .get("id")
        .and_then(|v| v.as_u64())
        .ok_or_else(|| McpError::InvalidParams("id parameter is required".into()))?;

    // Use provided cache or create temporary one (for stdio mode)
    let temp_cache;
    let cache = match api_key_cache {
        Some(c) => c,
        None => {
            temp_cache = ApiKeyCache::new(60, false);
            &temp_cache
        }
    };

    match crate::api_keys::revoke_api_key(adapter, id, cache) {
        Ok(true) => Ok(json!({"success": true, "id": id, "status": "revoked"})),
        Ok(false) => Ok(json!({"success": false, "id": id, "error": "API key not found"})),
        Err(e) => Err(McpError::Internal(e)),
    }
}

fn handle_admin_apikey_delete(
    params: Value,
    adapter: &Arc<IronBaseAdapter>,
    api_key_cache: Option<&ApiKeyCache>,
) -> Result<Value> {
    verify_admin_key(&params)?;
    let id = params
        .get("id")
        .and_then(|v| v.as_u64())
        .ok_or_else(|| McpError::InvalidParams("id parameter is required".into()))?;

    // Use provided cache or create temporary one (for stdio mode)
    let temp_cache;
    let cache = match api_key_cache {
        Some(c) => c,
        None => {
            temp_cache = ApiKeyCache::new(60, false);
            &temp_cache
        }
    };

    match crate::api_keys::delete_api_key(adapter, id, cache) {
        Ok(true) => Ok(json!({"success": true, "id": id, "status": "deleted"})),
        Ok(false) => Ok(json!({"success": false, "id": id, "error": "API key not found"})),
        Err(e) => Err(McpError::Internal(e)),
    }
}
