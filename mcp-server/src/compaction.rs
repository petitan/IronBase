//! Auto-compaction module — automatic storage bloat management
//!
//! The schedule (periodic checkpoint, bloat-triggered compaction) is the
//! core's (`ironbase_core::spawn_maintenance`); this module gives it the
//! server's target, whose compactions run as jobs.
//!
//! # Architecture
//!
//! - `AutoCompactConfig`: TOML-deserializable configuration (`[compaction]`)
//! - `ServerMaintenance`: the server's `MaintenanceTarget` (job-based compaction)
//! - `spawn_server_maintenance()`: starts the schedule (HTTP and stdio mode)
//! - `CompactGuard`: RAII guard for adapter compacting flag (shared with admin.rs)
//! - `run_compact_job()`: Background compact job execution (shared with admin.rs)

use crate::adapter::IronBaseAdapter;
use crate::jobs::{JobManager, JobType};
use serde_json::json;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;

// ============================================================================
// Configuration (TOML-deserializable)
// ============================================================================

fn default_true() -> bool {
    true
}
fn default_bloat_ratio() -> f64 {
    2.0
}
fn default_min_file_mb() -> u64 {
    100
}
fn default_check_interval() -> u64 {
    300
}
fn default_cooldown() -> u64 {
    1800
}

/// Auto-compaction configuration.
///
/// Loaded from `[compaction]` section in config.toml.
/// All fields have sensible defaults — works out of the box.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct AutoCompactConfig {
    /// Enable auto-compaction (default: true)
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Trigger compact when file_size / estimated_live >= this ratio (default: 2.0)
    #[serde(default = "default_bloat_ratio")]
    pub bloat_ratio_threshold: f64,
    /// Skip compact if file size < this many MB (default: 100)
    #[serde(default = "default_min_file_mb")]
    pub min_file_size_mb: u64,
    /// Seconds between wastage checks (default: 300 = 5 min)
    #[serde(default = "default_check_interval")]
    pub check_interval_secs: u64,
    /// Minimum seconds between two compactions (default: 1800 = 30 min)
    #[serde(default = "default_cooldown")]
    pub cooldown_secs: u64,
}

impl Default for AutoCompactConfig {
    fn default() -> Self {
        Self {
            enabled: default_true(),
            bloat_ratio_threshold: default_bloat_ratio(),
            min_file_size_mb: default_min_file_mb(),
            check_interval_secs: default_check_interval(),
            cooldown_secs: default_cooldown(),
        }
    }
}

impl AutoCompactConfig {
    /// The core's compaction policy for this configuration, `None` if disabled
    pub fn policy(&self) -> Option<ironbase_core::AutoCompactPolicy> {
        self.enabled.then(|| ironbase_core::AutoCompactPolicy {
            bloat_ratio_threshold: self.bloat_ratio_threshold,
            min_file_size_bytes: self.min_file_size_mb * 1024 * 1024,
            check_interval: std::time::Duration::from_secs(self.check_interval_secs),
            cooldown: std::time::Duration::from_secs(self.cooldown_secs),
        })
    }
}

// ============================================================================
// CompactGuard — RAII guard for compacting flag (shared with admin.rs)
// ============================================================================

/// RAII guard that clears the adapter's compacting flag on drop.
///
/// This ensures the flag is always cleared even if the compact thread panics.
pub struct CompactGuard {
    pub adapter: Arc<IronBaseAdapter>,
}

impl Drop for CompactGuard {
    fn drop(&mut self) {
        self.adapter.clear_compact_flag();
    }
}

// ============================================================================
// Background compact job execution (shared with admin.rs)
// ============================================================================

/// Run a compact job in the background.
///
/// Takes CompactGuard by reference — the guard is owned by the caller (closure)
/// and its Drop impl will clear the compacting flag when the closure exits.
///
/// `force_vector_rebuild`: `true` for an explicit operator `db_compact` (fully
/// reconstruct every HNSW graph), `false` for the automatic bloat-triggered
/// compact (orphan-gated rebuild only — cheap when nothing needs repair).
pub fn run_compact_job(
    job_id: &str,
    job_mgr: &JobManager,
    guard: &CompactGuard,
    shutdown_flag: &AtomicBool,
    force_vector_rebuild: bool,
) {
    let adapter = &guard.adapter;
    let start = Instant::now();
    job_mgr.update_progress(job_id, 0, None, "Starting compaction...");

    // Combined cancel flag: checked by CompactionConfig.is_cancelled()
    let combined_cancel = Arc::new(AtomicBool::new(false));
    let config = ironbase_core::storage::CompactionConfig::new()
        .with_cancel_flag(combined_cancel.clone())
        .with_force_vector_rebuild(force_vector_rebuild);

    let result = adapter.compact_nonblocking(&config, &|processed, total| {
        // Cancel propagation: job cancel OR shutdown → combined_cancel
        if shutdown_flag.load(Ordering::Relaxed) || job_mgr.is_cancelled(job_id) {
            combined_cancel.store(true, Ordering::Relaxed);
        }
        // Progress update → JobManager
        let total_usize = if total > 0 {
            Some(total as usize)
        } else {
            None
        };
        job_mgr.update_progress(
            job_id,
            processed as usize,
            total_usize,
            &format!("Scanning documents... {}/{}", processed, total),
        );
    });

    // Result handling
    match result {
        Ok(stats) => {
            let duration_ms = start.elapsed().as_millis() as u64;
            let space_freed = stats.size_before.saturating_sub(stats.size_after);

            // Calibrate last_compact_size for future bloat_ratio calculations
            adapter.set_last_compact_size(stats.size_after);

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

// ============================================================================
// The server's maintenance target
// ============================================================================

/// The periodic checkpoint every 60 s (MongoDB-style), as the HTTP server
/// always ran it; the stdio server now runs it too
const CHECKPOINT_INTERVAL: std::time::Duration = std::time::Duration::from_secs(60);

/// The server as a `MaintenanceTarget`: checkpoints through the adapter (which
/// follows `switch_database`), compactions as jobs (progress, cancellation,
/// joined on shutdown)
pub struct ServerMaintenance {
    pub adapter: Arc<IronBaseAdapter>,
    pub job_manager: Option<Arc<JobManager>>,
}

impl ironbase_core::MaintenanceTarget for ServerMaintenance {
    fn periodic_checkpoint(&self) -> ironbase_core::Result<ironbase_core::PeriodicCheckpointStats> {
        self.adapter.periodic_checkpoint()
    }

    fn storage_wastage(&self) -> ironbase_core::storage::StorageWastage {
        self.adapter.compute_wastage()
    }

    fn start_compaction(&self, _cancel: &Arc<AtomicBool>, done: Box<dyn FnOnce() + Send>) -> bool {
        let adapter = &self.adapter;
        if adapter.is_compacting() {
            tracing::debug!("Auto-compact skipped: compaction already in progress");
            return false;
        }
        let Some(job_mgr) = &self.job_manager else {
            tracing::debug!("Auto-compact skipped: no job manager");
            return false;
        };
        if !job_mgr.can_start_job() {
            tracing::debug!("Auto-compact skipped: max concurrent jobs reached");
            return false;
        }
        // Acquire compacting flag atomically
        if !adapter.try_start_compact() {
            tracing::debug!("Auto-compact skipped: CAS failed (concurrent compact)");
            return false;
        }

        let guard = CompactGuard {
            adapter: adapter.clone(),
        };
        let (job_id, _job) = job_mgr.create_job(JobType::Compact);
        let job_mgr_clone = job_mgr.clone();
        let job_id_clone = job_id.clone();
        // The job stops with the server (its shutdown flag), not with the
        // maintenance schedule: shutdown joins it like every job
        let shutdown_flag = job_mgr.shutdown_flag();
        let handle = std::thread::spawn(move || {
            // Automatic (bloat-triggered) compact: orphan-gated vector rebuild only,
            // so a routine startup compact does not pay for a full HNSW rebuild.
            run_compact_job(&job_id_clone, &job_mgr_clone, &guard, &shutdown_flag, false);
            done();
        });
        job_mgr.register_thread(job_id.clone(), handle);
        tracing::info!(job_id = %job_id, "Auto-compact triggered");
        true
    }
}

/// Start the server's background maintenance (HTTP and stdio mode alike).
/// Stop the handle before closing the adapter.
pub fn spawn_server_maintenance(
    adapter: Arc<IronBaseAdapter>,
    job_manager: Option<Arc<JobManager>>,
    config: &AutoCompactConfig,
) -> Option<ironbase_core::MaintenanceHandle> {
    let maintenance = ironbase_core::MaintenanceConfig {
        checkpoint_interval: Some(CHECKPOINT_INTERVAL),
        auto_compact: config.policy(),
    };
    if maintenance.auto_compact.is_none() {
        tracing::info!("Auto-compaction disabled by config");
    }
    let target = Arc::new(ServerMaintenance {
        adapter,
        job_manager,
    });
    match ironbase_core::spawn_maintenance(target, maintenance) {
        Ok(handle) => {
            tracing::info!("Background maintenance started (checkpoint every 60 s)");
            Some(handle)
        }
        Err(e) => {
            tracing::error!("Background maintenance could not start: {}", e);
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The `[compaction]` section maps onto the core's policy unchanged
    #[test]
    fn config_maps_onto_the_core_policy() {
        let p = AutoCompactConfig::default().policy().unwrap();
        assert_eq!(p.bloat_ratio_threshold, 2.0);
        assert_eq!(p.min_file_size_bytes, 100 * 1024 * 1024);
        assert_eq!(p.check_interval, std::time::Duration::from_secs(300));
        assert_eq!(p.cooldown, std::time::Duration::from_secs(1800));
        let off = AutoCompactConfig {
            enabled: false,
            ..AutoCompactConfig::default()
        };
        assert!(off.policy().is_none());
    }
}
