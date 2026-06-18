//! Auto-compaction module — automatic storage bloat management
//!
//! Monitors .mlite file size vs estimated live data and triggers
//! compaction when bloat exceeds configurable thresholds.
//!
//! # Architecture
//!
//! - `AutoCompactConfig`: TOML-deserializable configuration
//! - `AutoCompactState`: Runtime state (timers, cooldowns)
//! - `CompactGuard`: RAII guard for adapter compacting flag (shared with admin.rs)
//! - `run_compact_job()`: Background compact job execution (shared with admin.rs)
//! - `auto_compact_check()`: Wastage check + trigger logic
//! - `spawn_auto_compact_timer()`: Background thread for stdio mode

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

// ============================================================================
// Runtime state
// ============================================================================

/// Runtime state for auto-compaction timer.
pub struct AutoCompactState {
    config: AutoCompactConfig,
    last_check: Instant,
    last_compact_completed: Option<Instant>,
}

impl AutoCompactState {
    pub fn new(config: AutoCompactConfig) -> Self {
        Self {
            config,
            last_check: Instant::now(),
            last_compact_completed: None,
        }
    }

    /// Has enough time elapsed since the last check?
    pub fn should_check(&mut self) -> bool {
        if !self.config.enabled {
            return false;
        }
        let elapsed = self.last_check.elapsed().as_secs();
        if elapsed >= self.config.check_interval_secs {
            self.last_check = Instant::now();
            true
        } else {
            false
        }
    }

    /// Should we trigger compaction given the current wastage?
    ///
    /// Three gates:
    /// 1. bloat_ratio >= threshold
    /// 2. file_size >= min_file_size_mb
    /// 3. cooldown elapsed since last compact
    ///
    /// No file-size/RAM gate: the freeze that motivated one (2026-06-16) was a
    /// page-cache-management bug, fixed structurally by the page-cache-bounded
    /// compaction I/O (`storage/io.rs`, posix_fadvise(DONTNEED)+sync_file_range)
    /// which bounds the resident footprint to ~chunk_size regardless of file
    /// size. A RAM gate on top of that only disabled auto-compaction for large
    /// files that are now safe to compact — a capability regression.
    ///
    /// NON-GOAL (known, accepted): on non-Linux the page-cache hints in
    /// `storage/io.rs` are no-ops, so a large auto-compaction's OS buffer-cache
    /// footprint is not explicitly bounded there. This is acceptable because the
    /// freeze was only ever observed on Linux/WSL2 (the prod platform, where Fix B
    /// applies) and macOS/Windows manage the unified buffer cache differently. If a
    /// non-Linux host ever runs auto-compaction on a near-RAM-sized file, port the
    /// hints (F_NOCACHE / Windows cache flush) rather than re-adding a RAM gate.
    pub fn should_compact(&self, bloat_ratio: f64, file_size_bytes: u64) -> bool {
        // Gate 1: bloat ratio
        if bloat_ratio < self.config.bloat_ratio_threshold {
            return false;
        }

        // Gate 2: minimum file size
        let min_bytes = self.config.min_file_size_mb * 1024 * 1024;
        if file_size_bytes < min_bytes {
            return false;
        }

        // Gate 3: cooldown
        if let Some(last) = self.last_compact_completed {
            if last.elapsed().as_secs() < self.config.cooldown_secs {
                return false;
            }
        }

        true
    }

    /// Mark that a compaction has completed.
    pub fn mark_compact_completed(&mut self) {
        self.last_compact_completed = Some(Instant::now());
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
// Auto-compact check + trigger
// ============================================================================

/// Check storage wastage and trigger compaction if thresholds are exceeded.
///
/// Called periodically by the timer task (HTTP: tokio task, stdio: background thread).
/// Returns true if compaction was triggered.
pub fn auto_compact_check(
    adapter: &Arc<IronBaseAdapter>,
    job_manager: &Option<Arc<JobManager>>,
    state: &Arc<parking_lot::Mutex<AutoCompactState>>,
) -> bool {
    let wastage = adapter.compute_wastage();

    let should_compact = {
        let state_guard = state.lock();
        state_guard.should_compact(wastage.bloat_ratio, wastage.file_size_bytes)
    };

    tracing::debug!(
        file_size_mb = wastage.file_size_bytes / (1024 * 1024),
        estimated_live_mb = wastage.estimated_live_bytes / (1024 * 1024),
        bloat_ratio = format!("{:.2}", wastage.bloat_ratio),
        total_writes = wastage.total_writes,
        total_live = wastage.total_live,
        dead_writes = wastage.dead_writes,
        should_compact,
        "Storage wastage check"
    );

    if !should_compact {
        return false;
    }

    // Check preconditions for compact
    if adapter.is_compacting() {
        tracing::debug!("Auto-compact skipped: compaction already in progress");
        return false;
    }

    let job_mgr = match job_manager {
        Some(mgr) => mgr,
        None => {
            tracing::debug!("Auto-compact skipped: no job manager");
            return false;
        }
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
    let shutdown_flag = job_mgr.shutdown_flag();
    let state_clone = state.clone();

    let handle = std::thread::spawn(move || {
        // Automatic (bloat-triggered) compact: orphan-gated vector rebuild only,
        // so a routine startup compact does not pay for a full HNSW rebuild.
        run_compact_job(&job_id_clone, &job_mgr_clone, &guard, &shutdown_flag, false);
        // Mark compact completed for cooldown tracking
        state_clone.lock().mark_compact_completed();
    });

    job_mgr.register_thread(job_id.clone(), handle);

    tracing::info!(
        job_id = %job_id,
        bloat_ratio = format!("{:.2}", wastage.bloat_ratio),
        file_size_mb = wastage.file_size_bytes / (1024 * 1024),
        dead_writes = wastage.dead_writes,
        "Auto-compact triggered"
    );

    true
}

// ============================================================================
// Background timer for stdio mode
// ============================================================================

/// Spawn a background thread that periodically checks storage wastage
/// and triggers auto-compaction when thresholds are exceeded.
///
/// For stdio mode (no tokio runtime). Uses std::thread + sleep loop.
/// Returns the JoinHandle for cleanup.
pub fn spawn_auto_compact_timer(
    adapter: Arc<IronBaseAdapter>,
    job_manager: Option<Arc<JobManager>>,
    config: AutoCompactConfig,
    shutdown_flag: Arc<AtomicBool>,
) -> Option<std::thread::JoinHandle<()>> {
    if !config.enabled {
        tracing::info!("Auto-compaction disabled by config");
        return None;
    }

    let check_interval = std::time::Duration::from_secs(config.check_interval_secs);
    let state = Arc::new(parking_lot::Mutex::new(AutoCompactState::new(config)));

    let handle = std::thread::Builder::new()
        .name("auto-compact-timer".to_string())
        .spawn(move || {
            // Initial delay — let the server warm up
            std::thread::sleep(check_interval);

            while !shutdown_flag.load(Ordering::Relaxed) {
                let should = { state.lock().should_check() };
                if should {
                    auto_compact_check(&adapter, &job_manager, &state);
                }

                // Sleep in small increments to check shutdown flag responsively
                let sleep_step = std::time::Duration::from_secs(5);
                let mut remaining = check_interval;
                while remaining > std::time::Duration::ZERO {
                    if shutdown_flag.load(Ordering::Relaxed) {
                        return;
                    }
                    let step = remaining.min(sleep_step);
                    std::thread::sleep(step);
                    remaining = remaining.saturating_sub(step);
                }
            }
        })
        .ok();

    if handle.is_some() {
        tracing::info!("Auto-compact timer started (stdio mode)");
    }

    handle
}

#[cfg(test)]
mod tests {
    use super::*;

    const GB: u64 = 1024 * 1024 * 1024;

    fn state() -> AutoCompactState {
        AutoCompactState::new(AutoCompactConfig::default())
    }

    // --- should_compact gate composition ------------------------------------

    #[test]
    fn should_compact_allows_oversized_file_at_infinite_bloat() {
        // Legacy v5 docs.mlite reproduction: never-compacted → bloat_ratio=+inf,
        // file 7.1GB ≥ 100MB min, no cooldown — all gates pass → auto-compact
        // fires. The page-cache-bounded compaction I/O makes a large compaction
        // safe; there is no RAM gate (it was a capability regression).
        let s = state();
        assert!(s.should_compact(f64::INFINITY, 7 * GB + GB / 10));
    }

    #[test]
    fn should_compact_allows_bloated_file() {
        let s = state();
        // bloat 3.0 ≥ 2.0, file 7GB ≥ 100MB → compact regardless of host RAM
        assert!(s.should_compact(3.0, 7 * GB));
    }

    #[test]
    fn should_compact_still_honors_bloat_and_min_size_gates() {
        let s = state();
        // low bloat → no compact
        assert!(!s.should_compact(1.5, 7 * GB));
        // tiny file (< 100MB) → no compact even at infinite bloat
        assert!(!s.should_compact(f64::INFINITY, 10 * 1024 * 1024));
    }
}
