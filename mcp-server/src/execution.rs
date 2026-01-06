//! Unified execution context for MCP request processing
//!
//! This module consolidates timeout (deadline) and cancellation handling
//! into a single thread-local context, replacing the previous separate modules:
//! - `request_deadline.rs` (REQUEST_DEADLINE thread-local)
//! - Parts of `cancellation.rs` (CANCEL_FLAG thread-local)
//!
//! ## Design
//!
//! Each MCP request sets an `ExecutionContext` at the start of processing.
//! The adapter and tool handlers can then access the context via
//! `current_execution_context()` to create `ironbase_core::ExecutionContext`
//! for cooperative cancellation and timeout checking.
//!
//! ## Usage
//!
//! ```ignore
//! // In HTTP handler (start of request):
//! let ctx = ExecutionContext::new(Some(deadline), Some(cancel_flag));
//! let _guard = set_execution_context(ctx);
//! // ... process request ...
//! // Guard drops here, clearing thread-local
//!
//! // In adapter (during query execution):
//! let ctx = current_execution_context();
//! let core_ctx = ironbase_core::ExecutionContext::new()
//!     .with_deadline(ctx.as_ref().and_then(|c| c.deadline))
//!     .with_cancel_flag(ctx.as_ref().and_then(|c| c.cancel_flag.clone()));
//! ```

use std::cell::Cell;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;

thread_local! {
    static EXECUTION_CONTEXT: Cell<Option<ExecutionContext>> = const { Cell::new(None) };
}

/// Execution context for the current MCP request
///
/// Contains deadline (timeout) and cancellation flag for cooperative
/// cancellation of long-running operations.
#[derive(Clone)]
pub struct ExecutionContext {
    /// Deadline (timeout) for the request
    pub deadline: Option<Instant>,
    /// Cancellation flag from RequestTracker
    pub cancel_flag: Option<Arc<AtomicBool>>,
}

impl ExecutionContext {
    /// Create a new execution context
    pub fn new(deadline: Option<Instant>, cancel_flag: Option<Arc<AtomicBool>>) -> Self {
        Self {
            deadline,
            cancel_flag,
        }
    }

    /// Check if the request has been cancelled
    pub fn is_cancelled(&self) -> bool {
        self.cancel_flag
            .as_ref()
            .map(|f| f.load(Ordering::Relaxed))
            .unwrap_or(false)
    }

    /// Check if the deadline has been exceeded
    pub fn is_timed_out(&self) -> bool {
        self.deadline
            .map(|dl| Instant::now() >= dl)
            .unwrap_or(false)
    }
}

/// Set execution context for the current thread
///
/// Returns a guard that restores the previous context on drop.
/// Typically called at the start of MCP request processing.
///
/// # Example
///
/// ```ignore
/// let ctx = ExecutionContext::new(Some(deadline), Some(cancel_flag));
/// let _guard = set_execution_context(ctx);
/// // ... process request ...
/// // guard drops here, restoring previous context
/// ```
pub fn set_execution_context(ctx: ExecutionContext) -> ExecutionGuard {
    let previous = EXECUTION_CONTEXT.with(|cell| {
        let prev = cell.take();
        cell.set(Some(ctx));
        prev
    });
    ExecutionGuard { previous }
}

/// Get the current execution context (cloned)
///
/// Returns `None` if no context is set for this thread.
/// Use this in adapter methods to create `ironbase_core::ExecutionContext`.
pub fn current_execution_context() -> Option<ExecutionContext> {
    EXECUTION_CONTEXT.with(|cell| {
        let ctx = cell.take();
        let result = ctx.clone();
        cell.set(ctx);
        result
    })
}

/// Get current deadline (shorthand)
pub fn current_deadline() -> Option<Instant> {
    current_execution_context().and_then(|ctx| ctx.deadline)
}

/// Get current cancel flag (shorthand)
pub fn current_cancel_flag() -> Option<Arc<AtomicBool>> {
    current_execution_context().and_then(|ctx| ctx.cancel_flag)
}

/// Check if current request is cancelled (shorthand)
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
    current_execution_context()
        .map(|ctx| ctx.is_cancelled())
        .unwrap_or(false)
}

/// RAII guard to restore previous execution context on drop
pub struct ExecutionGuard {
    previous: Option<ExecutionContext>,
}

impl Drop for ExecutionGuard {
    fn drop(&mut self) {
        EXECUTION_CONTEXT.with(|cell| cell.set(self.previous.take()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_execution_context_basic() {
        // Initially no context
        assert!(current_execution_context().is_none());
        assert!(!is_cancelled());
        assert!(current_deadline().is_none());

        // Set context
        let deadline = Instant::now() + std::time::Duration::from_secs(10);
        let cancel_flag = Arc::new(AtomicBool::new(false));
        {
            let ctx = ExecutionContext::new(Some(deadline), Some(cancel_flag.clone()));
            let _guard = set_execution_context(ctx);

            // Context should be available
            assert!(current_execution_context().is_some());
            assert!(!is_cancelled());
            assert!(current_deadline().is_some());

            // Set cancelled
            cancel_flag.store(true, Ordering::SeqCst);
            assert!(is_cancelled());
        }

        // After guard drops, context should be cleared
        assert!(current_execution_context().is_none());
        assert!(!is_cancelled());
    }

    #[test]
    fn test_execution_context_nested() {
        let ctx1 = ExecutionContext::new(
            Some(Instant::now() + std::time::Duration::from_secs(10)),
            None,
        );
        let _guard1 = set_execution_context(ctx1);

        assert!(current_deadline().is_some());

        {
            // Nested context
            let ctx2 = ExecutionContext::new(None, Some(Arc::new(AtomicBool::new(true))));
            let _guard2 = set_execution_context(ctx2);

            // Should see inner context
            assert!(current_deadline().is_none());
            assert!(is_cancelled());
        }

        // After inner guard drops, should see outer context
        assert!(current_deadline().is_some());
        assert!(!is_cancelled());
    }

    #[test]
    fn test_is_timed_out() {
        // Past deadline
        let ctx = ExecutionContext::new(
            Some(Instant::now() - std::time::Duration::from_secs(1)),
            None,
        );
        assert!(ctx.is_timed_out());

        // Future deadline
        let ctx = ExecutionContext::new(
            Some(Instant::now() + std::time::Duration::from_secs(10)),
            None,
        );
        assert!(!ctx.is_timed_out());

        // No deadline
        let ctx = ExecutionContext::new(None, None);
        assert!(!ctx.is_timed_out());
    }
}
