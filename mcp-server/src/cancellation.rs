//! Request cancellation support for MCP `notifications/cancelled`
//!
//! This module provides:
//! - `RequestTracker`: Thread-safe registry of in-flight requests
//! - Thread-local cancellation flag for cooperative checking in tools
//!
//! ## MCP Spec Compliance
//!
//! Per MCP spec (2024-11-05), the server SHOULD:
//! - Stop processing cancelled requests
//! - Free associated resources
//! - NOT send a response for cancelled requests
//!
//! The server MAY ignore cancellation if:
//! - The request is unknown
//! - Processing has already completed
//! - The request cannot be cancelled

use dashmap::DashMap;
use std::cell::Cell;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// Thread-safe in-flight request registry
///
/// Tracks active requests by their JSON-RPC ID, allowing cancellation
/// via `notifications/cancelled`.
pub struct RequestTracker {
    in_flight: DashMap<String, Arc<AtomicBool>>,
}

impl RequestTracker {
    /// Create a new request tracker
    pub fn new() -> Self {
        Self {
            in_flight: DashMap::new(),
        }
    }

    /// Register a new request, returns cancellation flag
    ///
    /// The returned `Arc<AtomicBool>` should be:
    /// 1. Stored in thread-local via `set_cancel_flag()`
    /// 2. Checked periodically in long-running operations via `is_cancelled()`
    pub fn start(&self, request_id: &str) -> Arc<AtomicBool> {
        let flag = Arc::new(AtomicBool::new(false));
        self.in_flight.insert(request_id.to_string(), flag.clone());
        flag
    }

    /// Unregister a completed request
    ///
    /// Called when request processing finishes (success or error).
    /// Safe to call multiple times or with unknown IDs.
    pub fn finish(&self, request_id: &str) {
        self.in_flight.remove(request_id);
    }

    /// Cancel a request by ID (MCP `notifications/cancelled`)
    ///
    /// Returns `true` if request was found and cancelled.
    /// Returns `false` if request was unknown or already completed (per MCP spec, this is OK).
    pub fn cancel(&self, request_id: &str, reason: Option<&str>) -> bool {
        if let Some((_, flag)) = self.in_flight.remove(request_id) {
            flag.store(true, Ordering::SeqCst);
            tracing::info!(
                request_id = request_id,
                reason = reason.unwrap_or("none"),
                "Request cancelled via notifications/cancelled"
            );
            true
        } else {
            // Per MCP spec: MAY ignore if unknown/completed
            tracing::debug!(
                request_id = request_id,
                "Cancel request ignored: unknown or already completed"
            );
            false
        }
    }

    /// Get current in-flight request count (for metrics/debugging)
    pub fn in_flight_count(&self) -> usize {
        self.in_flight.len()
    }
}

impl Default for RequestTracker {
    fn default() -> Self {
        Self::new()
    }
}

// Thread-local cancellation flag for cooperative checking
thread_local! {
    static CANCEL_FLAG: Cell<Option<Arc<AtomicBool>>> = const { Cell::new(None) };
}

/// Set cancellation flag for current thread
///
/// Returns a guard that clears the flag on drop.
/// Typical usage:
/// ```ignore
/// let flag = tracker.start(&request_id);
/// let _guard = set_cancel_flag(flag);
/// // ... do work, periodically call is_cancelled() ...
/// // guard drops here, clearing thread-local
/// ```
pub fn set_cancel_flag(flag: Arc<AtomicBool>) -> CancelGuard {
    CANCEL_FLAG.set(Some(flag));
    CancelGuard
}

/// Check if current request is cancelled
///
/// Call this periodically in long-running operations:
/// ```ignore
/// for (i, doc) in documents.iter().enumerate() {
///     if i % 1000 == 0 && is_cancelled() {
///         return Err(McpError::cancelled("Request was cancelled"));
///     }
///     process(doc);
/// }
/// ```
pub fn is_cancelled() -> bool {
    CANCEL_FLAG.with(|cell| {
        // We need to temporarily take the value to read it
        let flag = cell.take();
        let result = flag
            .as_ref()
            .map(|f| f.load(Ordering::Relaxed))
            .unwrap_or(false);
        cell.set(flag);
        result
    })
}

/// RAII guard to clear thread-local cancellation flag on drop
pub struct CancelGuard;

impl Drop for CancelGuard {
    fn drop(&mut self) {
        CANCEL_FLAG.set(None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_request_tracker_basic() {
        let tracker = RequestTracker::new();

        // Start a request
        let flag = tracker.start("req-1");
        assert_eq!(tracker.in_flight_count(), 1);
        assert!(!flag.load(Ordering::Relaxed));

        // Cancel it
        assert!(tracker.cancel("req-1", Some("user requested")));
        assert!(flag.load(Ordering::Relaxed));
        assert_eq!(tracker.in_flight_count(), 0);

        // Cancel unknown request (should return false, not panic)
        assert!(!tracker.cancel("req-unknown", None));
    }

    #[test]
    fn test_request_tracker_finish() {
        let tracker = RequestTracker::new();

        let _flag = tracker.start("req-1");
        assert_eq!(tracker.in_flight_count(), 1);

        tracker.finish("req-1");
        assert_eq!(tracker.in_flight_count(), 0);

        // Finish unknown request (should not panic)
        tracker.finish("req-unknown");
    }

    #[test]
    fn test_thread_local_cancel_flag() {
        // Initially not cancelled
        assert!(!is_cancelled());

        // Set a flag
        let flag = Arc::new(AtomicBool::new(false));
        {
            let _guard = set_cancel_flag(flag.clone());

            // Not cancelled yet
            assert!(!is_cancelled());

            // Set cancelled
            flag.store(true, Ordering::SeqCst);
            assert!(is_cancelled());
        }

        // Guard dropped, should be cleared
        assert!(!is_cancelled());
    }

    #[test]
    fn test_cancel_guard_drop() {
        let flag = Arc::new(AtomicBool::new(true));

        {
            let _guard = set_cancel_flag(flag.clone());
            assert!(is_cancelled());
        }

        // After guard drops, is_cancelled should return false
        // (because thread-local is cleared, not because flag changed)
        assert!(!is_cancelled());
    }
}
