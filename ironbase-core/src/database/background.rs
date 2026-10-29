//! Background maintenance: the periodic checkpoint and the bloat-triggered
//! compaction, one schedule for every entry point.
//!
//! A long-running process keeps its indexes on disk by checkpointing (else a
//! crash rebuilds them from the documents) and reclaims the space of updates
//! and deletes by compacting. The MCP server used to schedule both itself, in
//! two copies (HTTP: tokio tasks; stdio: a thread), and the stdio copy never
//! checkpointed; an embedded database (Rust, Python) had no schedule at all.
//!
//! [`spawn_maintenance`] runs the schedule on a thread for any
//! [`MaintenanceTarget`]: a `DatabaseCore<StorageEngine>` directly (the
//! compaction runs on the maintenance thread), or a server's own target that
//! starts its compactions as jobs. The returned [`MaintenanceHandle`] stops
//! the thread (and cancels a compaction it runs) when stopped or dropped;
//! stop it before closing the database.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crate::error::Result;
use crate::storage::{CheckpointStats, CompactionConfig, StorageEngine, StorageWastage};

use super::DatabaseCore;

/// When the background maintenance runs
#[derive(Debug, Clone)]
pub struct MaintenanceConfig {
    /// Periodic checkpoint (B+ tree indexes, metadata, WAL); `None` = off.
    /// Default 60 s, MongoDB-style.
    pub checkpoint_interval: Option<Duration>,
    /// Bloat-triggered compaction; `None` = off
    pub auto_compact: Option<AutoCompactPolicy>,
}

impl Default for MaintenanceConfig {
    fn default() -> Self {
        Self {
            checkpoint_interval: Some(Duration::from_secs(60)),
            auto_compact: Some(AutoCompactPolicy::default()),
        }
    }
}

/// When a compaction is worth it
#[derive(Debug, Clone)]
pub struct AutoCompactPolicy {
    /// Compact when file size / estimated live data reaches this (default 2.0)
    pub bloat_ratio_threshold: f64,
    /// Never compact a smaller file (default 100 MiB)
    pub min_file_size_bytes: u64,
    /// How often the wastage is checked (default 5 min)
    pub check_interval: Duration,
    /// Minimum time from a finished compaction to the next (default 30 min)
    pub cooldown: Duration,
}

impl Default for AutoCompactPolicy {
    fn default() -> Self {
        Self {
            bloat_ratio_threshold: 2.0,
            min_file_size_bytes: 100 * 1024 * 1024,
            check_interval: Duration::from_secs(300),
            cooldown: Duration::from_secs(1800),
        }
    }
}

impl AutoCompactPolicy {
    /// Three gates: bloat ratio, minimum file size, cooldown since the last
    /// finished compaction.
    ///
    /// No file-size/RAM gate: the freeze that motivated one (2026-06-16) was a
    /// page-cache-management bug, fixed by the page-cache-bounded compaction
    /// I/O (`storage/io.rs`), which bounds the resident footprint regardless of
    /// file size. On non-Linux those hints are no-ops; the freeze was only
    /// observed on Linux/WSL2.
    pub fn should_compact(&self, wastage: &StorageWastage, since_last: Option<Duration>) -> bool {
        wastage.bloat_ratio >= self.bloat_ratio_threshold
            && wastage.file_size_bytes >= self.min_file_size_bytes
            && since_last.is_none_or(|elapsed| elapsed >= self.cooldown)
    }
}

/// What the maintenance thread works on
pub trait MaintenanceTarget: Send + Sync + 'static {
    /// One periodic checkpoint
    fn periodic_checkpoint(&self) -> Result<PeriodicCheckpointStats>;

    /// The current storage wastage
    fn storage_wastage(&self) -> StorageWastage;

    /// Start a compaction. Returns false if none could start (one is
    /// running, no capacity); else `done` must be called when it finishes,
    /// successfully or not (the cooldown counts from then). `cancel` is set
    /// when the maintenance stops; a compaction run on the calling thread
    /// must honour it.
    fn start_compaction(&self, cancel: &Arc<AtomicBool>, done: Box<dyn FnOnce() + Send>) -> bool;
}

/// Result of [`DatabaseCore::periodic_checkpoint`]
#[derive(Debug, Clone)]
pub struct PeriodicCheckpointStats {
    /// B+ tree indexes written
    pub indexes_flushed: usize,
    /// Metadata and WAL
    pub checkpoint: CheckpointStats,
}

impl DatabaseCore<StorageEngine> {
    /// The checkpoint to run periodically while the database is in use: the
    /// dirty B+ tree indexes, then metadata and WAL. Fulltext and fuzzy indexes
    /// are left to `checkpoint()` / `close()`: flushing them blocked
    /// `insert_one` for minutes under memory pressure. Both phases take the
    /// storage lock briefly, so reads and writes keep running.
    pub fn periodic_checkpoint(&self) -> Result<PeriodicCheckpointStats> {
        let indexes_flushed = self.flush_btree_indexes_counted()?;
        let checkpoint = self.checkpoint_wal_only()?;
        Ok(PeriodicCheckpointStats {
            indexes_flushed,
            checkpoint,
        })
    }
}

/// An embedded database maintains itself: the compaction runs on the
/// maintenance thread, cancelled when the maintenance stops
impl MaintenanceTarget for DatabaseCore<StorageEngine> {
    fn periodic_checkpoint(&self) -> Result<PeriodicCheckpointStats> {
        DatabaseCore::periodic_checkpoint(self)
    }

    fn storage_wastage(&self) -> StorageWastage {
        DatabaseCore::storage_wastage(self)
    }

    fn start_compaction(&self, cancel: &Arc<AtomicBool>, done: Box<dyn FnOnce() + Send>) -> bool {
        // Automatic compaction: orphan-gated vector rebuild only
        let config = CompactionConfig::new().with_cancel_flag(Arc::clone(cancel));
        match self.compact_nonblocking(&config, &|_, _| {}) {
            Ok(stats) => {
                self.set_last_compact_size(stats.size_after);
                crate::log_info!(
                    "Auto-compaction: {} -> {} bytes, {} documents kept",
                    stats.size_before,
                    stats.size_after,
                    stats.documents_kept
                );
            }
            Err(e) => crate::log_warn!("Auto-compaction failed: {}", e),
        }
        done();
        true
    }
}

/// The schedule's state: when each task is next due
#[derive(Debug)]
struct Schedule {
    config: MaintenanceConfig,
    next_checkpoint: Option<Instant>,
    next_compact_check: Option<Instant>,
}

impl Schedule {
    fn new(config: MaintenanceConfig, now: Instant) -> Self {
        // Neither runs at once: the process is starting up
        Schedule {
            next_checkpoint: config.checkpoint_interval.map(|i| now + i),
            next_compact_check: config.auto_compact.as_ref().map(|p| now + p.check_interval),
            config,
        }
    }

    /// The earliest due time, `None` if nothing is scheduled
    fn next_due(&self) -> Option<Instant> {
        match (self.next_checkpoint, self.next_compact_check) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
    }
}

/// Stops the maintenance thread: [`MaintenanceHandle::stop`], or drop
pub struct MaintenanceHandle {
    wake: Arc<(Mutex<bool>, Condvar)>,
    cancel: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl MaintenanceHandle {
    /// Stop the thread and wait for it; a compaction it runs is cancelled
    pub fn stop(mut self) {
        self.shut_down();
    }

    fn shut_down(&mut self) {
        self.cancel.store(true, Ordering::SeqCst);
        {
            let (stopped, condvar) = &*self.wake;
            *stopped.lock().unwrap_or_else(|e| e.into_inner()) = true;
            condvar.notify_all();
        }
        if let Some(thread) = self.thread.take() {
            if thread.join().is_err() {
                crate::log_warn!("Maintenance thread panicked");
            }
        }
    }
}

impl Drop for MaintenanceHandle {
    fn drop(&mut self) {
        self.shut_down();
    }
}

/// Run the maintenance schedule of `config` for `target` on a thread
pub fn spawn_maintenance<T: MaintenanceTarget + ?Sized>(
    target: Arc<T>,
    config: MaintenanceConfig,
) -> std::io::Result<MaintenanceHandle> {
    let wake = Arc::new((Mutex::new(false), Condvar::new()));
    let cancel = Arc::new(AtomicBool::new(false));
    let thread = {
        let wake = Arc::clone(&wake);
        let cancel = Arc::clone(&cancel);
        std::thread::Builder::new()
            .name("ironbase-maintenance".to_string())
            .spawn(move || run(&*target, config, &wake, &cancel))?
    };
    Ok(MaintenanceHandle {
        wake,
        cancel,
        thread: Some(thread),
    })
}

fn run<T: MaintenanceTarget + ?Sized>(
    target: &T,
    config: MaintenanceConfig,
    wake: &(Mutex<bool>, Condvar),
    cancel: &Arc<AtomicBool>,
) {
    let mut schedule = Schedule::new(config, Instant::now());
    // When the last compaction finished; set by the compaction's `done`
    let last_compact: Arc<Mutex<Option<Instant>>> = Arc::new(Mutex::new(None));
    let (stopped, condvar) = wake;
    loop {
        // Sleep until something is due, or the maintenance stops
        {
            let mut guard = stopped.lock().unwrap_or_else(|e| e.into_inner());
            loop {
                if *guard {
                    return;
                }
                let now = Instant::now();
                match schedule.next_due() {
                    Some(due) if due <= now => break,
                    Some(due) => {
                        guard = condvar
                            .wait_timeout(guard, due - now)
                            .unwrap_or_else(|e| e.into_inner())
                            .0;
                    }
                    None => {
                        guard = condvar.wait(guard).unwrap_or_else(|e| e.into_inner());
                    }
                }
            }
        }
        let now = Instant::now();

        if let (Some(due), Some(interval)) = (
            schedule.next_checkpoint,
            schedule.config.checkpoint_interval,
        ) {
            if due <= now {
                match target.periodic_checkpoint() {
                    Ok(stats) if stats.indexes_flushed > 0 => crate::log_info!(
                        "Periodic checkpoint: {} indexes flushed",
                        stats.indexes_flushed
                    ),
                    Ok(_) => {}
                    Err(e) => crate::log_warn!("Periodic checkpoint failed: {}", e),
                }
                schedule.next_checkpoint = Some(Instant::now() + interval);
            }
        }

        if let (Some(due), Some(policy)) = (
            schedule.next_compact_check,
            schedule.config.auto_compact.clone(),
        ) {
            if due <= now {
                let wastage = target.storage_wastage();
                let since_last = last_compact
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .map(|t| t.elapsed());
                if policy.should_compact(&wastage, since_last) {
                    crate::log_info!(
                        "Auto-compaction due: bloat ratio {:.2}, file {} MB",
                        wastage.bloat_ratio,
                        wastage.file_size_bytes / (1024 * 1024)
                    );
                    let finished = Arc::clone(&last_compact);
                    let done = Box::new(move || {
                        *finished.lock().unwrap_or_else(|e| e.into_inner()) = Some(Instant::now());
                    });
                    if !target.start_compaction(cancel, done) {
                        crate::log_debug!("Auto-compaction skipped: could not start");
                    }
                }
                schedule.next_compact_check = Some(Instant::now() + policy.check_interval);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    const MB: u64 = 1024 * 1024;

    fn wastage(bloat_ratio: f64, file_size_bytes: u64) -> StorageWastage {
        StorageWastage {
            file_size_bytes,
            estimated_live_bytes: 0,
            bloat_ratio,
            total_writes: 0,
            total_live: 0,
            dead_writes: 0,
        }
    }

    #[test]
    fn policy_gates() {
        let p = AutoCompactPolicy::default();
        // Never compacted (+inf) and large: compact, whatever the RAM
        assert!(p.should_compact(&wastage(f64::INFINITY, 7 * 1024 * MB), None));
        assert!(p.should_compact(&wastage(3.0, 200 * MB), None));
        assert!(!p.should_compact(&wastage(1.5, 7 * 1024 * MB), None));
        assert!(!p.should_compact(&wastage(f64::INFINITY, 10 * MB), None));
        // Cooldown from the last finished compaction
        assert!(!p.should_compact(&wastage(3.0, 200 * MB), Some(Duration::from_secs(60))));
        assert!(p.should_compact(&wastage(3.0, 200 * MB), Some(Duration::from_secs(1800))));
    }

    /// A target that counts calls and always wants a compaction
    #[derive(Default)]
    struct Counting {
        checkpoints: AtomicUsize,
        compactions: AtomicUsize,
        cancelled: AtomicBool,
    }

    impl MaintenanceTarget for Counting {
        fn periodic_checkpoint(&self) -> Result<PeriodicCheckpointStats> {
            self.checkpoints.fetch_add(1, Ordering::SeqCst);
            Ok(PeriodicCheckpointStats {
                indexes_flushed: 0,
                checkpoint: CheckpointStats::default(),
            })
        }
        fn storage_wastage(&self) -> StorageWastage {
            wastage(f64::INFINITY, u64::MAX)
        }
        fn start_compaction(
            &self,
            cancel: &Arc<AtomicBool>,
            done: Box<dyn FnOnce() + Send>,
        ) -> bool {
            self.compactions.fetch_add(1, Ordering::SeqCst);
            // A long compaction: runs until the maintenance stops
            while !cancel.load(Ordering::SeqCst) {
                std::thread::sleep(Duration::from_millis(5));
            }
            self.cancelled.store(true, Ordering::SeqCst);
            done();
            true
        }
    }

    #[test]
    fn checkpoints_on_its_interval_and_stops_promptly() {
        let target = Arc::new(Counting::default());
        let config = MaintenanceConfig {
            checkpoint_interval: Some(Duration::from_millis(20)),
            auto_compact: None,
        };
        let spawned = Instant::now();
        let handle = spawn_maintenance(Arc::clone(&target), config).unwrap();
        // Repeats: wait for the third checkpoint (a loaded CI runner may
        // starve the thread, so no fixed wall-clock count is asserted)
        while target.checkpoints.load(Ordering::SeqCst) < 3 {
            assert!(
                spawned.elapsed() < Duration::from_secs(10),
                "only {} checkpoints in 10 s at 20 ms",
                target.checkpoints.load(Ordering::SeqCst)
            );
            std::thread::sleep(Duration::from_millis(5));
        }
        let started = Instant::now();
        handle.stop();
        assert!(started.elapsed() < Duration::from_secs(1), "stop waited");
        // ...but never more often than its interval
        let n = target.checkpoints.load(Ordering::SeqCst);
        let max = (spawned.elapsed().as_millis() / 20) as usize;
        assert!(
            n <= max,
            "{n} checkpoints, at most {max} fit the 20 ms interval"
        );
        // None after the stop
        std::thread::sleep(Duration::from_millis(60));
        assert_eq!(target.checkpoints.load(Ordering::SeqCst), n);
    }

    #[test]
    fn nothing_runs_before_its_first_interval() {
        let target = Arc::new(Counting::default());
        let handle = spawn_maintenance(Arc::clone(&target), MaintenanceConfig::default()).unwrap();
        std::thread::sleep(Duration::from_millis(50));
        drop(handle);
        assert_eq!(target.checkpoints.load(Ordering::SeqCst), 0);
        assert_eq!(target.compactions.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn stopping_cancels_a_running_compaction() {
        let target = Arc::new(Counting::default());
        let config = MaintenanceConfig {
            checkpoint_interval: None,
            auto_compact: Some(AutoCompactPolicy {
                check_interval: Duration::from_millis(10),
                ..AutoCompactPolicy::default()
            }),
        };
        let handle = spawn_maintenance(Arc::clone(&target), config).unwrap();
        while target.compactions.load(Ordering::SeqCst) == 0 {
            std::thread::sleep(Duration::from_millis(5));
        }
        handle.stop();
        assert!(target.cancelled.load(Ordering::SeqCst));
        assert_eq!(target.compactions.load(Ordering::SeqCst), 1);
    }

    /// An embedded database: the periodic checkpoint persists a new B+ tree
    /// index entry, and an embedded database runs on its own schedule
    #[test]
    fn embedded_database_checkpoints_itself() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("m.mlite");
        let db = Arc::new(DatabaseCore::<StorageEngine>::open(&path).unwrap());
        db.collection("c")
            .unwrap()
            .create_index("k".to_string(), false, false)
            .unwrap();
        let wal = path.with_extension("wal");
        for i in 0..20 {
            db.insert_one(
                "c",
                std::collections::HashMap::from([
                    ("_id".to_string(), serde_json::json!(i)),
                    ("k".to_string(), serde_json::json!(i % 3)),
                ]),
            )
            .unwrap();
        }
        let wal_written = std::fs::metadata(&wal).map(|m| m.len()).unwrap_or(0);
        assert!(wal_written > 0);
        // Started after the writes, so the WAL above is not raced
        let config = MaintenanceConfig {
            checkpoint_interval: Some(Duration::from_millis(20)),
            auto_compact: None,
        };
        let handle = spawn_maintenance(Arc::clone(&db), config).unwrap();
        // The checkpoint clears the WAL
        let deadline = Instant::now() + Duration::from_secs(5);
        while std::fs::metadata(&wal).map(|m| m.len()).unwrap_or(0) > 0 {
            assert!(Instant::now() < deadline, "WAL never checkpointed");
            std::thread::sleep(Duration::from_millis(10));
        }
        handle.stop();
        db.close().unwrap();
    }
}
