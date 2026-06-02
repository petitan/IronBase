//! Admin tool handlers (require IRONBASE_ADMIN_KEY)
//!
//! Uses typed parameter structs for compile-time validation.

use crate::adapter::IronBaseAdapter;
use crate::api_keys::ApiKeyCache;
use crate::compaction::{run_compact_job, CompactGuard};
use crate::error::{McpError, Result};
use crate::jobs::{JobManager, JobType};
use crate::ServerInfo;
use serde_json::{json, Value};
use std::sync::Arc;

use super::params::{
    AdminApiKeyCreateParams, AdminApiKeyIdParams, AdminCollectionParams, AdminFlagsParams,
    AdminKeyParams, DbOpenParams, ParseParams,
};

/// Dispatch admin tool calls
pub fn dispatch(
    name: &str,
    params: Value,
    adapter: &Arc<IronBaseAdapter>,
    api_key_cache: Option<&ApiKeyCache>,
    server_info: Option<&ServerInfo>,
    job_manager: &Option<Arc<JobManager>>,
) -> Result<Value> {
    match name {
        "db_open" => handle_db_open(params, adapter),
        "db_stats" => handle_db_stats(adapter, server_info),
        "db_compact" => handle_db_compact(adapter, job_manager),
        "db_checkpoint" => handle_db_checkpoint(adapter),
        "admin_list_all_collections" => handle_admin_list_all_collections(params, adapter),
        "admin_create_system_collection" => handle_admin_create_system_collection(params, adapter),
        "admin_set_collection_flags" => handle_admin_set_collection_flags(params, adapter),
        "admin_drop_protected" => handle_admin_drop_protected(params, adapter),
        "admin_apikey_create" => handle_admin_apikey_create(params, adapter, api_key_cache),
        "admin_apikey_list" => handle_admin_apikey_list(params, adapter),
        "admin_apikey_revoke" => handle_admin_apikey_revoke(params, adapter, api_key_cache),
        "admin_apikey_delete" => handle_admin_apikey_delete(params, adapter, api_key_cache),
        _ => Err(McpError::invalid_params(format!(
            "Unknown admin tool: {}",
            name
        ))),
    }
}

fn handle_db_open(params: Value, adapter: &Arc<IronBaseAdapter>) -> Result<Value> {
    let p: DbOpenParams = DbOpenParams::parse(params)?;
    let new_path = adapter.switch_database(&p.path, p.create)?;

    Ok(json!({
        "success": true,
        "path": new_path,
        "message": if p.create { "Database created" } else { "Database opened" }
    }))
}

fn handle_db_stats(
    adapter: &Arc<IronBaseAdapter>,
    server_info: Option<&ServerInfo>,
) -> Result<Value> {
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

    // Add storage wastage section
    let wastage = adapter.compute_wastage();
    if let Some(obj) = stats.as_object_mut() {
        obj.insert(
            "storage_wastage".to_string(),
            json!({
                "file_size_bytes": wastage.file_size_bytes,
                "file_size_mb": wastage.file_size_bytes / (1024 * 1024),
                "estimated_live_bytes": wastage.estimated_live_bytes,
                "estimated_live_mb": wastage.estimated_live_bytes / (1024 * 1024),
                "bloat_ratio": if wastage.bloat_ratio.is_infinite() {
                    "infinity (never compacted)".to_string()
                } else {
                    format!("{:.2}", wastage.bloat_ratio)
                },
                "total_writes": wastage.total_writes,
                "total_live": wastage.total_live,
                "dead_writes": wastage.dead_writes,
            }),
        );
    }

    Ok(stats)
}

fn handle_db_compact(
    adapter: &Arc<IronBaseAdapter>,
    job_manager: &Option<Arc<JobManager>>,
) -> Result<Value> {
    // 1. Singleton guard — reject if already running
    if adapter.is_compacting() {
        return Err(McpError::invalid_params(
            "Compaction already in progress. Use embed_job_status to check status.",
        ));
    }

    // 2. Job manager required for async dispatch
    let job_mgr = job_manager
        .as_ref()
        .ok_or_else(|| McpError::internal("Job manager not available"))?;

    if !job_mgr.can_start_job() {
        return Err(McpError::invalid_params(
            "Maximum concurrent jobs reached. Wait for running jobs to complete.",
        ));
    }

    // 3. Acquire guard atomically (CAS: false→true)
    if !adapter.try_start_compact() {
        return Err(McpError::invalid_params("Compaction already in progress"));
    }

    // RAII guard — ensures clear_compact_flag() is called even if thread panics
    // or if create_job() panics below (before thread spawn)
    let guard = CompactGuard {
        adapter: adapter.clone(),
    };

    let (job_id, _job) = job_mgr.create_job(JobType::Compact);

    // 4. Clone + spawn background thread (guard moves into thread)
    let job_mgr_clone = job_mgr.clone();
    let job_id_clone = job_id.clone();
    let shutdown_flag = job_mgr.shutdown_flag();

    let handle = std::thread::spawn(move || {
        // guard is moved here — Drop will fire when this closure exits
        // (whether normally, via early return, or via panic)
        run_compact_job(&job_id_clone, &job_mgr_clone, &guard, &shutdown_flag);
    });

    job_mgr.register_thread(job_id.clone(), handle);

    tracing::info!("Compact job {} started in background", job_id);

    Ok(json!({
        "status": "started",
        "job_id": job_id,
        "message": "Compaction started in background. Use embed_job_status to check progress."
    }))
}

// NOTE: CompactGuard and run_compact_job() are defined in crate::compaction
// and shared between admin.rs (manual db_compact) and auto-compact timer.

fn handle_db_checkpoint(adapter: &Arc<IronBaseAdapter>) -> Result<Value> {
    adapter.checkpoint()
}

fn handle_admin_list_all_collections(
    params: Value,
    adapter: &Arc<IronBaseAdapter>,
) -> Result<Value> {
    AdminKeyParams::parse(params)?; // validate params; admin key gated centrally
    let collections = adapter.list_all_collections();
    Ok(json!({"collections": collections, "count": collections.len()}))
}

fn handle_admin_create_system_collection(
    params: Value,
    adapter: &Arc<IronBaseAdapter>,
) -> Result<Value> {
    let p: AdminCollectionParams = AdminCollectionParams::parse(params)?;
    adapter.create_system_collection(&p.collection)?;
    Ok(
        json!({"success": true, "collection": p.collection, "flags": {"is_system": true, "protected": true, "hidden": false}}),
    )
}

fn handle_admin_set_collection_flags(
    params: Value,
    adapter: &Arc<IronBaseAdapter>,
) -> Result<Value> {
    let p: AdminFlagsParams = AdminFlagsParams::parse(params)?;
    adapter.set_collection_flags(&p.collection, p.is_system, p.protected, p.hidden)?;
    Ok(
        json!({"success": true, "collection": p.collection, "flags": {"is_system": p.is_system, "protected": p.protected, "hidden": p.hidden}}),
    )
}

fn handle_admin_drop_protected(params: Value, adapter: &Arc<IronBaseAdapter>) -> Result<Value> {
    let p: AdminCollectionParams = AdminCollectionParams::parse(params)?;
    adapter.force_drop_collection(&p.collection)?;
    Ok(json!({"success": true, "dropped": p.collection}))
}

fn handle_admin_apikey_create(
    params: Value,
    adapter: &Arc<IronBaseAdapter>,
    api_key_cache: Option<&ApiKeyCache>,
) -> Result<Value> {
    let p: AdminApiKeyCreateParams = AdminApiKeyCreateParams::parse(params)?;

    // Use provided cache or create temporary one (for stdio mode)
    let temp_cache;
    let cache = match api_key_cache {
        Some(c) => c,
        None => {
            temp_cache = ApiKeyCache::new(60, false);
            &temp_cache
        }
    };

    match crate::api_keys::create_api_key(adapter, &p.name, cache) {
        Ok(api_key) => Ok(json!({
            "success": true,
            "id": api_key._id,
            "key": api_key.key,
            "name": api_key.name,
            "created_at": api_key.created_at,
            "note": "Save this key now - it cannot be retrieved later!"
        })),
        Err(e) => Err(McpError::internal(e)),
    }
}

fn handle_admin_apikey_list(params: Value, adapter: &Arc<IronBaseAdapter>) -> Result<Value> {
    AdminKeyParams::parse(params)?; // validate params; admin key gated centrally
    match crate::api_keys::list_api_keys(adapter) {
        Ok(keys) => Ok(json!({
            "success": true,
            "keys": keys,
            "count": keys.len()
        })),
        Err(e) => Err(McpError::internal(e)),
    }
}

fn handle_admin_apikey_revoke(
    params: Value,
    adapter: &Arc<IronBaseAdapter>,
    api_key_cache: Option<&ApiKeyCache>,
) -> Result<Value> {
    let p: AdminApiKeyIdParams = AdminApiKeyIdParams::parse(params)?;

    // Use provided cache or create temporary one (for stdio mode)
    let temp_cache;
    let cache = match api_key_cache {
        Some(c) => c,
        None => {
            temp_cache = ApiKeyCache::new(60, false);
            &temp_cache
        }
    };

    match crate::api_keys::revoke_api_key(adapter, p.id, cache) {
        Ok(true) => Ok(json!({"success": true, "id": p.id, "status": "revoked"})),
        Ok(false) => Ok(json!({"success": false, "id": p.id, "error": "API key not found"})),
        Err(e) => Err(McpError::internal(e)),
    }
}

fn handle_admin_apikey_delete(
    params: Value,
    adapter: &Arc<IronBaseAdapter>,
    api_key_cache: Option<&ApiKeyCache>,
) -> Result<Value> {
    let p: AdminApiKeyIdParams = AdminApiKeyIdParams::parse(params)?;

    // Use provided cache or create temporary one (for stdio mode)
    let temp_cache;
    let cache = match api_key_cache {
        Some(c) => c,
        None => {
            temp_cache = ApiKeyCache::new(60, false);
            &temp_cache
        }
    };

    match crate::api_keys::delete_api_key(adapter, p.id, cache) {
        Ok(true) => Ok(json!({"success": true, "id": p.id, "status": "deleted"})),
        Ok(false) => Ok(json!({"success": false, "id": p.id, "error": "API key not found"})),
        Err(e) => Err(McpError::internal(e)),
    }
}
