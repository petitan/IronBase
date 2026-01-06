//! Unified Execution Context for Long-Running Operations
//!
//! This module provides a unified way to handle cancellation and timeouts
//! across all long-running database operations (find, aggregate, search, etc.).
//!
//! # Example
//!
//! ```rust,ignore
//! use ironbase_core::ExecutionContext;
//! use std::time::{Duration, Instant};
//!
//! // Create context with deadline
//! let ctx = ExecutionContext::new()
//!     .with_deadline(Instant::now() + Duration::from_secs(60));
//!
//! // In a loop, check periodically
//! for (i, item) in items.iter().enumerate() {
//!     ctx.maybe_check(i)?;  // Returns error if cancelled/timed out
//!     process(item);
//! }
//! ```

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;

use crate::error::{IronBaseError, Result};

/// Unified execution context for all long-running operations.
///
/// Supports cooperative cancellation via:
/// - **Deadline**: Time-based cancellation (e.g., HTTP request timeout)
/// - **Cancel flag**: External signal (e.g., client disconnect, user abort)
///
/// Operations should call `maybe_check()` periodically (every N iterations)
/// to check for cancellation. The default check frequency is 100 iterations.
#[derive(Clone, Default)]
pub struct ExecutionContext {
    /// Deadline for time-based cancellation
    deadline: Option<Instant>,
    /// External cancel flag (e.g., from HTTP request tracker)
    cancel_flag: Option<Arc<AtomicBool>>,
    /// Check frequency (every N iterations)
    check_frequency: usize,
}

impl ExecutionContext {
    /// Create a new execution context with default settings.
    ///
    /// Default check frequency is 100 iterations.
    pub fn new() -> Self {
        Self {
            deadline: None,
            cancel_flag: None,
            check_frequency: 100,
        }
    }

    /// Set the deadline for this operation.
    ///
    /// The operation will return `IronBaseError::Timeout` if the deadline
    /// is exceeded when `check_cancelled()` is called.
    pub fn with_deadline(mut self, deadline: Instant) -> Self {
        self.deadline = Some(deadline);
        self
    }

    /// Set the deadline from an optional value.
    pub fn with_deadline_opt(mut self, deadline: Option<Instant>) -> Self {
        self.deadline = deadline;
        self
    }

    /// Set the external cancel flag.
    ///
    /// The operation will return `IronBaseError::Cancelled` if the flag
    /// is set to `true` when `check_cancelled()` is called.
    pub fn with_cancel_flag(mut self, flag: Arc<AtomicBool>) -> Self {
        self.cancel_flag = Some(flag);
        self
    }

    /// Set the cancel flag from an optional value.
    pub fn with_cancel_flag_opt(mut self, flag: Option<Arc<AtomicBool>>) -> Self {
        self.cancel_flag = flag;
        self
    }

    /// Set the check frequency (every N iterations).
    ///
    /// Lower values provide faster cancellation response but add overhead.
    /// Default is 100.
    pub fn with_check_frequency(mut self, freq: usize) -> Self {
        self.check_frequency = if freq == 0 { 1 } else { freq };
        self
    }

    /// Get the deadline if set.
    pub fn deadline(&self) -> Option<Instant> {
        self.deadline
    }

    /// Get the cancel flag if set.
    pub fn cancel_flag(&self) -> Option<&Arc<AtomicBool>> {
        self.cancel_flag.as_ref()
    }

    /// Get the check frequency.
    pub fn check_frequency(&self) -> usize {
        self.check_frequency
    }

    /// Check if the operation should be cancelled.
    ///
    /// Returns:
    /// - `Ok(())` if the operation can continue
    /// - `Err(IronBaseError::Cancelled)` if the cancel flag is set
    /// - `Err(IronBaseError::Timeout)` if the deadline has passed
    ///
    /// Cancel flag is checked first (faster atomic load vs. clock read).
    pub fn check_cancelled(&self) -> Result<()> {
        // Check cancel flag first (faster - just atomic load)
        if let Some(ref flag) = self.cancel_flag {
            if flag.load(Ordering::Relaxed) {
                return Err(IronBaseError::Cancelled(
                    "Operation cancelled by client".to_string(),
                ));
            }
        }

        // Check deadline (requires clock read)
        if let Some(deadline) = self.deadline {
            if Instant::now() >= deadline {
                return Err(IronBaseError::Timeout(
                    "Operation timed out (deadline exceeded)".to_string(),
                ));
            }
        }

        Ok(())
    }

    /// Check if we should check at this iteration.
    ///
    /// Returns `true` every `check_frequency` iterations.
    #[inline]
    pub fn should_check_at(&self, iteration: usize) -> bool {
        iteration % self.check_frequency == 0
    }

    /// Convenience method: check if should check AND do the check.
    ///
    /// Call this in a loop to periodically check for cancellation:
    ///
    /// ```rust,ignore
    /// for (i, item) in items.iter().enumerate() {
    ///     ctx.maybe_check(i)?;
    ///     process(item);
    /// }
    /// ```
    #[inline]
    pub fn maybe_check(&self, iteration: usize) -> Result<()> {
        if self.should_check_at(iteration) {
            self.check_cancelled()?;
        }
        Ok(())
    }

    /// Check if this context has any cancellation mechanism configured.
    pub fn has_cancellation(&self) -> bool {
        self.deadline.is_some() || self.cancel_flag.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn test_default_context() {
        let ctx = ExecutionContext::new();
        assert!(ctx.deadline.is_none());
        assert!(ctx.cancel_flag.is_none());
        assert_eq!(ctx.check_frequency, 100);
        assert!(!ctx.has_cancellation());
    }

    #[test]
    fn test_with_deadline() {
        let deadline = Instant::now() + Duration::from_secs(60);
        let ctx = ExecutionContext::new().with_deadline(deadline);
        assert_eq!(ctx.deadline, Some(deadline));
        assert!(ctx.has_cancellation());
    }

    #[test]
    fn test_with_cancel_flag() {
        let flag = Arc::new(AtomicBool::new(false));
        let ctx = ExecutionContext::new().with_cancel_flag(flag.clone());
        assert!(ctx.cancel_flag.is_some());
        assert!(ctx.has_cancellation());
    }

    #[test]
    fn test_check_cancelled_ok() {
        let ctx = ExecutionContext::new().with_deadline(Instant::now() + Duration::from_secs(60));
        assert!(ctx.check_cancelled().is_ok());
    }

    #[test]
    fn test_check_cancelled_timeout() {
        let ctx = ExecutionContext::new().with_deadline(Instant::now() - Duration::from_secs(1)); // Past deadline
        let result = ctx.check_cancelled();
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), IronBaseError::Timeout(_)));
    }

    #[test]
    fn test_check_cancelled_flag() {
        let flag = Arc::new(AtomicBool::new(true)); // Already cancelled
        let ctx = ExecutionContext::new().with_cancel_flag(flag);
        let result = ctx.check_cancelled();
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), IronBaseError::Cancelled(_)));
    }

    #[test]
    fn test_should_check_at() {
        let ctx = ExecutionContext::new().with_check_frequency(10);
        assert!(ctx.should_check_at(0));
        assert!(!ctx.should_check_at(1));
        assert!(!ctx.should_check_at(9));
        assert!(ctx.should_check_at(10));
        assert!(ctx.should_check_at(100));
    }

    #[test]
    fn test_maybe_check() {
        let flag = Arc::new(AtomicBool::new(false));
        let ctx = ExecutionContext::new()
            .with_cancel_flag(flag.clone())
            .with_check_frequency(10);

        // Should pass when not cancelled
        assert!(ctx.maybe_check(0).is_ok());
        assert!(ctx.maybe_check(10).is_ok());

        // Set cancel flag
        flag.store(true, Ordering::Relaxed);

        // Should still pass at non-check iterations
        assert!(ctx.maybe_check(11).is_ok());

        // Should fail at check iteration
        assert!(ctx.maybe_check(20).is_err());
    }
}
