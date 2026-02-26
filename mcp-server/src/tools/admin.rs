//! Admin tool handlers (require IRONBASE_ADMIN_KEY)
//!
//! Uses typed parameter structs for compile-time validation.

use crate::adapter::IronBaseAdapter;
use crate::api_keys::ApiKeyCache;
use crate::error::{McpError, Result};
use crate::jobs::{JobManager, JobType};
use crate::ServerInfo;
use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use super::helpers::verify_admin_key_opt;
use super::params::{
    AdminApiKeyCreateParams, AdminApiKeyIdParams, AdminCollectionParams, AdminFlagsParams,
    AdminKeyParams, DbOpenParams, ParseParams,
};

// ============================================================================
// RAII Guard for compacting flag
// ============================================================================

/// RAII guard that clears the adapter's compacting flag on drop.
///
/// This ensures the flag is always cleared even if the compact thread panics.
/// Created by handle_db_compact(), moved into the spawned thread.
struct CompactGuard {
    adapter: Arc<IronBaseAdapter>,
}

impl Drop for CompactGuard {
    fn drop(&mut self) {
        self.adapter.clear_compact_flag();
    }
}

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
    let job_mgr = job_manager.as_ref().ok_or_else(|| {
        McpError::internal("Job manager not available")
    })?;

    if !job_mgr.can_start_job() {
        return Err(McpError::invalid_params(
            "Maximum concurrent jobs reached. Wait for running jobs to complete.",
        ));
    }

    // 3. Acquire guard atomically (CAS: false→true)
    if !adapter.try_start_compact() {
        return Err(McpError::invalid_params(
            "Compaction already in progress",
        ));
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
        run_compact_job(
            &job_id_clone,
            &job_mgr_clone,
            &guard,
            &shutdown_flag,
        );
    });

    job_mgr.register_thread(job_id.clone(), handle);

    tracing::info!("Compact job {} started in background", job_id);

    Ok(json!({
        "status": "started",
        "job_id": job_id,
        "message": "Compaction started in background. Use embed_job_status to check progress."
    }))
}

/// Background compact job execution
///
/// Pattern follows run_backfill_job() in auto_embed.rs.
///
/// Takes CompactGuard by reference — the guard is owned by the caller (closure)
/// and its Drop impl will clear the compacting flag when the closure exits,
/// whether normally or via panic. No manual clear_compact_flag() needed.
fn run_compact_job(
    job_id: &str,
    job_mgr: &JobManager,
    guard: &CompactGuard,
    shutdown_flag: &AtomicBool,
) {
    let adapter = &guard.adapter;
    let start = std::time::Instant::now();
    job_mgr.update_progress(job_id, 0, None, "Starting compaction...");

    // Combined cancel flag: checked by CompactionConfig.is_cancelled()
    let combined_cancel = Arc::new(AtomicBool::new(false));
    let config = ironbase_core::storage::CompactionConfig::new()
        .with_cancel_flag(combined_cancel.clone());

    let result = adapter.compact_nonblocking(
        &config,
        &|processed, total| {
            // Cancel propagation: job cancel OR shutdown → combined_cancel
            if shutdown_flag.load(Ordering::Relaxed)
                || job_mgr.is_cancelled(job_id)
            {
                combined_cancel.store(true, Ordering::Relaxed);
            }
            // Progress update → JobManager
            let total_usize = if total > 0 { Some(total as usize) } else { None };
            job_mgr.update_progress(
                job_id,
                processed as usize,
                total_usize,
                &format!("Scanning documents... {}/{}", processed, total),
            );
        },
    );

    // Result handling
    match result {
        Ok(stats) => {
            let duration_ms = start.elapsed().as_millis() as u64;
            let space_freed = stats.size_before.saturating_sub(stats.size_after);
            tracing::info!(
                job_id,
                duration_ms,
                size_before = stats.size_before,
                size_after = stats.size_after,
                space_freed,
                docs_scanned = stats.documents_scanned,
                docs_kept = stats.documents_kept,
                tombstones = stats.tombstones_removed,
                "Compact job completed"
            );
            job_mgr.complete_job(
                job_id,
                json!({
                    "success": true,
                    "size_before_bytes": stats.size_before,
                    "size_after_bytes": stats.size_after,
                    "space_freed_bytes": space_freed,
                    "documents_scanned": stats.documents_scanned,
                    "documents_kept": stats.documents_kept,
                    "tombstones_removed": stats.tombstones_removed,
                    "compression_ratio": format!("{:.1}%", stats.compression_ratio()),
                    "duration_ms": duration_ms,
                }),
            );
        }
        Err(e) if e.to_string().contains("cancelled") => {
            tracing::info!(job_id, "Compact job cancelled");
            job_mgr.complete_job(
                job_id,
                json!({
                    "status": "cancelled",
                    "message": e.to_string(),
                }),
            );
        }
        Err(e) => {
            tracing::error!(job_id, error = %e, "Compact job failed");
            job_mgr.fail_job(job_id, e.to_string());
        }
    }

    // NOTE: No manual clear_compact_flag() needed!
    // The CompactGuard's Drop impl handles it when the caller closure exits.
}

fn handle_db_checkpoint(adapter: &Arc<IronBaseAdapter>) -> Result<Value> {
    adapter.checkpoint()
}

fn handle_admin_list_all_collections(
    params: Value,
    adapter: &Arc<IronBaseAdapter>,
) -> Result<Value> {
    let p: AdminKeyParams = AdminKeyParams::parse(params)?;
    verify_admin_key_opt(p.admin_key.as_deref())?;
    let collections = adapter.list_all_collections();
    Ok(json!({"collections": collections, "count": collections.len()}))
}

fn handle_admin_create_system_collection(
    params: Value,
    adapter: &Arc<IronBaseAdapter>,
) -> Result<Value> {
    let p: AdminCollectionParams = AdminCollectionParams::parse(params)?;
    verify_admin_key_opt(p.admin_key.as_deref())?;
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
    verify_admin_key_opt(p.admin_key.as_deref())?;
    adapter.set_collection_flags(&p.collection, p.is_system, p.protected, p.hidden)?;
    Ok(
        json!({"success": true, "collection": p.collection, "flags": {"is_system": p.is_system, "protected": p.protected, "hidden": p.hidden}}),
    )
}

fn handle_admin_drop_protected(params: Value, adapter: &Arc<IronBaseAdapter>) -> Result<Value> {
    let p: AdminCollectionParams = AdminCollectionParams::parse(params)?;
    verify_admin_key_opt(p.admin_key.as_deref())?;
    adapter.force_drop_collection(&p.collection)?;
    Ok(json!({"success": true, "dropped": p.collection}))
}

fn handle_admin_apikey_create(
    params: Value,
    adapter: &Arc<IronBaseAdapter>,
    api_key_cache: Option<&ApiKeyCache>,
) -> Result<Value> {
    let p: AdminApiKeyCreateParams = AdminApiKeyCreateParams::parse(params)?;
    verify_admin_key_opt(p.admin_key.as_deref())?;

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
    let p: AdminKeyParams = AdminKeyParams::parse(params)?;
    verify_admin_key_opt(p.admin_key.as_deref())?;
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
    verify_admin_key_opt(p.admin_key.as_deref())?;

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
    verify_admin_key_opt(p.admin_key.as_deref())?;

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
