//! Request cancellation support for MCP `notifications/cancelled`
//!
//! This module provides:
//! - `RequestTracker`: Thread-safe registry of in-flight requests
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
//!
//! ## Note
//!
//! Thread-local cancellation flag handling has been moved to `execution.rs`
//! which provides a unified `ExecutionContext` for both deadline and cancellation.

use dashmap::DashMap;
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
    /// The returned `Arc<AtomicBool>` should be passed to `ExecutionContext`
    /// via `set_execution_context()` in the HTTP handler.
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
    fn test_multiple_requests() {
        let tracker = RequestTracker::new();

        let flag1 = tracker.start("req-1");
        let flag2 = tracker.start("req-2");
        let flag3 = tracker.start("req-3");
        assert_eq!(tracker.in_flight_count(), 3);

        // Cancel one
        tracker.cancel("req-2", None);
        assert!(!flag1.load(Ordering::Relaxed));
        assert!(flag2.load(Ordering::Relaxed));
        assert!(!flag3.load(Ordering::Relaxed));
        assert_eq!(tracker.in_flight_count(), 2);

        // Finish another
        tracker.finish("req-1");
        assert_eq!(tracker.in_flight_count(), 1);
    }
}
