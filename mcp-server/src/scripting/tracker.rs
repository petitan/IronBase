//! Active script tracking for graceful shutdown.
//!
//! Tracks running scripts and provides mechanisms for graceful termination
//! during server shutdown.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// Tracks active script executions for graceful shutdown.
///
/// When the server receives a shutdown signal:
/// 1. Mark shutdown flag
/// 2. Reject new script executions
/// 3. Wait for active scripts to complete (with timeout)
///
/// # Example
///
/// ```rust,ignore
/// let tracker = Arc::new(ActiveScriptTracker::new());
///
/// // In script execution:
/// let guard = tracker.track()?;
/// // ... execute script ...
/// // guard dropped automatically
///
/// // In shutdown handler:
/// tracker.signal_shutdown();
/// tracker.wait_for_completion(Duration::from_secs(30)).await;
/// ```
#[derive(Debug)]
pub struct ActiveScriptTracker {
    /// Number of currently executing scripts.
    active_count: AtomicUsize,

    /// Shutdown flag - when true, reject new scripts.
    shutdown: AtomicBool,
}

impl Default for ActiveScriptTracker {
    fn default() -> Self {
        Self::new()
    }
}

impl ActiveScriptTracker {
    /// Create a new tracker.
    pub fn new() -> Self {
        Self {
            active_count: AtomicUsize::new(0),
            shutdown: AtomicBool::new(false),
        }
    }

    /// Check if new scripts should be accepted.
    ///
    /// Returns `false` after `signal_shutdown()` is called.
    pub fn can_accept(&self) -> bool {
        !self.shutdown.load(Ordering::SeqCst)
    }

    /// Track a new script execution.
    ///
    /// Returns `Some(ScriptExecutionGuard)` if the script can proceed,
    /// or `None` if the server is shutting down.
    ///
    /// The guard automatically decrements the counter when dropped.
    ///
    /// # Returns
    ///
    /// - `Some(guard)` - Script can execute, guard tracks lifetime
    /// - `None` - Server is shutting down, reject the script
    pub fn track(self: &Arc<Self>) -> Option<ScriptExecutionGuard> {
        // Check shutdown flag first
        if self.shutdown.load(Ordering::SeqCst) {
            return None;
        }

        // Increment counter
        self.active_count.fetch_add(1, Ordering::SeqCst);

        // Double-check after increment (race condition guard)
        if self.shutdown.load(Ordering::SeqCst) {
            // Shutdown happened between check and increment
            self.active_count.fetch_sub(1, Ordering::SeqCst);
            return None;
        }

        Some(ScriptExecutionGuard {
            tracker: Arc::clone(self),
        })
    }

    /// Get the current count of active scripts.
    pub fn active_count(&self) -> usize {
        self.active_count.load(Ordering::SeqCst)
    }

    /// Check if shutdown has been signaled.
    pub fn is_shutdown(&self) -> bool {
        self.shutdown.load(Ordering::SeqCst)
    }

    /// Signal shutdown - stop accepting new scripts.
    ///
    /// After calling this:
    /// - `can_accept()` returns `false`
    /// - `track()` returns `None`
    ///
    /// Existing scripts continue to run until completion or timeout.
    pub fn signal_shutdown(&self) {
        self.shutdown.store(true, Ordering::SeqCst);
    }

    /// Wait for all active scripts to complete.
    ///
    /// Polls the active count at 100ms intervals until:
    /// - All scripts complete (returns `true`)
    /// - Timeout expires (returns `false`)
    ///
    /// # Arguments
    ///
    /// * `timeout` - Maximum time to wait
    ///
    /// # Returns
    ///
    /// - `true` - All scripts completed gracefully
    /// - `false` - Timeout expired with scripts still running
    pub async fn wait_for_completion(&self, timeout: Duration) -> bool {
        let deadline = tokio::time::Instant::now() + timeout;
        let poll_interval = Duration::from_millis(100);

        loop {
            let count = self.active_count();
            if count == 0 {
                tracing::info!("All scripts completed gracefully");
                return true;
            }

            if tokio::time::Instant::now() >= deadline {
                tracing::warn!(
                    "Shutdown timeout: {} scripts still running, forcing termination",
                    count
                );
                return false;
            }

            tracing::debug!("Waiting for {} active scripts to complete...", count);
            tokio::time::sleep(poll_interval).await;
        }
    }

    /// Wait for completion (sync version).
    ///
    /// Uses `std::thread::sleep` instead of async.
    /// Prefer `wait_for_completion` in async contexts.
    pub fn wait_for_completion_sync(&self, timeout: Duration) -> bool {
        let deadline = std::time::Instant::now() + timeout;
        let poll_interval = Duration::from_millis(100);

        loop {
            let count = self.active_count();
            if count == 0 {
                return true;
            }

            if std::time::Instant::now() >= deadline {
                return false;
            }

            std::thread::sleep(poll_interval);
        }
    }
}

/// RAII guard for tracking script execution lifetime.
///
/// Created by `ActiveScriptTracker::track()`.
/// Automatically decrements the active count when dropped.
///
/// # Example
///
/// ```rust,ignore
/// let guard = tracker.track()?;
/// // Script executes here...
/// drop(guard); // or just let it go out of scope
/// // Counter decremented
/// ```
#[derive(Debug)]
pub struct ScriptExecutionGuard {
    tracker: Arc<ActiveScriptTracker>,
}

impl Drop for ScriptExecutionGuard {
    fn drop(&mut self) {
        self.tracker.active_count.fetch_sub(1, Ordering::SeqCst);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tracker_basic() {
        let tracker = Arc::new(ActiveScriptTracker::new());
        assert!(tracker.can_accept());
        assert_eq!(tracker.active_count(), 0);

        {
            let _guard = tracker.track().unwrap();
            assert_eq!(tracker.active_count(), 1);
        }
        // Guard dropped
        assert_eq!(tracker.active_count(), 0);
    }

    #[test]
    fn test_tracker_multiple() {
        let tracker = Arc::new(ActiveScriptTracker::new());

        let guard1 = tracker.track().unwrap();
        let guard2 = tracker.track().unwrap();
        assert_eq!(tracker.active_count(), 2);

        drop(guard1);
        assert_eq!(tracker.active_count(), 1);

        drop(guard2);
        assert_eq!(tracker.active_count(), 0);
    }

    #[test]
    fn test_tracker_shutdown_rejects_new() {
        let tracker = Arc::new(ActiveScriptTracker::new());

        // Start one script
        let _guard = tracker.track().unwrap();
        assert_eq!(tracker.active_count(), 1);

        // Signal shutdown
        tracker.signal_shutdown();
        assert!(!tracker.can_accept());
        assert!(tracker.is_shutdown());

        // New scripts should be rejected
        assert!(tracker.track().is_none());

        // Existing script still counted
        assert_eq!(tracker.active_count(), 1);
    }

    #[test]
    fn test_tracker_sync_wait() {
        let tracker = Arc::new(ActiveScriptTracker::new());

        // No active scripts - should return immediately
        assert!(tracker.wait_for_completion_sync(Duration::from_millis(100)));

        // With active script
        let _guard = tracker.track().unwrap();
        // Timeout should fail (script still running)
        assert!(!tracker.wait_for_completion_sync(Duration::from_millis(50)));
    }
}
