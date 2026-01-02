//! Resource limits for script execution.
//!
//! Defines constants and structures for controlling script resource usage
//! to prevent OOM (Out of Memory) and DoS attacks.
//!
//! ## Dynamic Limits
//!
//! Limits can be calculated dynamically based on available system memory
//! using [`DynamicLimits::calculate()`]. The [`LimitsManager`] provides
//! thread-safe access to these limits with periodic refresh capability.

use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

// ============================================================
// Operation Limits
// ============================================================

/// Default maximum number of Rhai operations per script execution.
///
/// Prevents infinite loops and excessive CPU usage.
/// Each Rhai statement/expression counts as one operation.
pub const DEFAULT_MAX_OPERATIONS: u64 = 1_000_000;

/// Default script execution timeout in milliseconds.
///
/// Scripts running longer than this will be cancelled.
pub const DEFAULT_TIMEOUT_MS: u64 = 60_000; // 60 seconds

// ============================================================
// Memory Limits (Min/Max for Dynamic Calculation)
// ============================================================

/// Minimum script result size in bytes.
///
/// Dynamic limits will never go below this value.
pub const MIN_RESULT_SIZE_BYTES: usize = 5 * 1024 * 1024; // 5 MB

/// Maximum script result size in bytes.
///
/// Dynamic limits will never exceed this value.
/// Prevents scripts from returning excessively large results.
pub const MAX_RESULT_SIZE_BYTES: usize = 100 * 1024 * 1024; // 100 MB

/// Default result size (used when memory info unavailable).
pub const DEFAULT_RESULT_SIZE_BYTES: usize = 10 * 1024 * 1024; // 10 MB

/// Maximum size of a single log entry in bytes.
///
/// Log entries exceeding this will be truncated.
/// This is a fixed value, not dynamically calculated.
pub const MAX_SINGLE_LOG_ENTRY_BYTES: usize = 64 * 1024; // 64 KB

/// Minimum total size of all log entries combined.
pub const MIN_TOTAL_LOG_BYTES: usize = 512 * 1024; // 512 KB

/// Maximum total size of all log entries combined.
pub const MAX_TOTAL_LOG_BYTES: usize = 10 * 1024 * 1024; // 10 MB

/// Default total log size (used when memory info unavailable).
pub const DEFAULT_TOTAL_LOG_BYTES: usize = 1024 * 1024; // 1 MB

/// Maximum number of log entries per script execution.
pub const MAX_LOG_ENTRIES: usize = 10_000;

// ============================================================
// Database Query Limits
// ============================================================

/// Minimum limit for `db_find()`.
///
/// Dynamic limits will never go below this value.
pub const MIN_FIND_DOCUMENTS: usize = 1_000;

/// Default limit for `db_find()` when no limit is specified.
///
/// Prevents accidental full collection scans from exhausting memory.
pub const MAX_FIND_DOCUMENTS: usize = 10_000;

/// Maximum limit for `db_find()` in dynamic mode.
pub const DYNAMIC_MAX_FIND_DOCUMENTS: usize = 500_000;

/// Absolute maximum limit for `db_find()` even with explicit limit.
///
/// Hard cap that cannot be exceeded regardless of user request.
pub const ABSOLUTE_MAX_FIND_DOCUMENTS: usize = 1_000_000;

// ============================================================
// ScriptLimits Structure
// ============================================================

/// Configuration for script execution resource limits.
///
/// Controls CPU, memory, and time usage during script execution.
/// All limits are enforced during execution.
///
/// # Example
///
/// ```rust,ignore
/// let limits = ScriptLimits {
///     timeout_ms: 30_000, // 30 second timeout
///     max_result_size: 5 * 1024 * 1024, // 5 MB max result
///     ..Default::default()
/// };
/// engine.run_with_limits(code, params, limits).await?;
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScriptLimits {
    /// Maximum number of Rhai operations.
    ///
    /// Default: 1,000,000
    pub max_operations: u64,

    /// Execution timeout in milliseconds.
    ///
    /// Default: 60,000 (60 seconds)
    pub timeout_ms: u64,

    /// Maximum result size in bytes.
    ///
    /// Default: 10 MB
    pub max_result_size: usize,

    /// Default document limit for `db_find()`.
    ///
    /// Default: 10,000
    pub max_find_documents: usize,

    /// Maximum size of single log entry.
    ///
    /// Default: 64 KB
    pub max_log_entry_size: usize,

    /// Maximum total size of all logs.
    ///
    /// Default: 1 MB
    pub max_total_log_size: usize,
}

impl Default for ScriptLimits {
    fn default() -> Self {
        Self {
            max_operations: DEFAULT_MAX_OPERATIONS,
            timeout_ms: DEFAULT_TIMEOUT_MS,
            max_result_size: DEFAULT_RESULT_SIZE_BYTES,
            max_find_documents: MAX_FIND_DOCUMENTS,
            max_log_entry_size: MAX_SINGLE_LOG_ENTRY_BYTES,
            max_total_log_size: DEFAULT_TOTAL_LOG_BYTES,
        }
    }
}

impl ScriptLimits {
    /// Create limits with custom max operations.
    ///
    /// # Arguments
    ///
    /// * `max_ops` - Maximum number of Rhai operations
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let limits = ScriptLimits::with_max_operations(500_000);
    /// ```
    pub fn with_max_operations(max_ops: u64) -> Self {
        Self {
            max_operations: max_ops,
            ..Default::default()
        }
    }

    /// Create limits with custom timeout.
    ///
    /// # Arguments
    ///
    /// * `timeout_ms` - Timeout in milliseconds
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let limits = ScriptLimits::with_timeout(30_000); // 30 seconds
    /// ```
    pub fn with_timeout(timeout_ms: u64) -> Self {
        Self {
            timeout_ms,
            ..Default::default()
        }
    }

    /// Check if a result size exceeds the limit.
    ///
    /// # Arguments
    ///
    /// * `size` - Size in bytes to check
    ///
    /// # Returns
    ///
    /// `true` if size exceeds limit, `false` otherwise
    pub fn result_exceeds_limit(&self, size: usize) -> bool {
        size > self.max_result_size
    }

    /// Get the effective find limit (capped at absolute max).
    ///
    /// # Arguments
    ///
    /// * `requested` - Requested limit from user
    ///
    /// # Returns
    ///
    /// The effective limit, capped at `ABSOLUTE_MAX_FIND_DOCUMENTS`
    pub fn effective_find_limit(&self, requested: Option<usize>) -> usize {
        match requested {
            Some(req) => req.min(ABSOLUTE_MAX_FIND_DOCUMENTS),
            None => self.max_find_documents.min(ABSOLUTE_MAX_FIND_DOCUMENTS),
        }
    }
}

// ============================================================
// Dynamic Limits (Memory-Based)
// ============================================================

/// Dynamic limits calculated from available system memory.
///
/// Instead of using fixed hardcoded limits, this calculates
/// appropriate limits based on the current system's available RAM.
///
/// # Formula
///
/// - `max_result_size`: 5% of available RAM, clamped to [5 MB, 100 MB]
/// - `max_total_log_size`: 0.5% of available RAM, clamped to [512 KB, 10 MB]
/// - `max_find_documents`: result_size / 1 KB (avg doc size), clamped to [1K, 500K]
///
/// # Example
///
/// ```rust,ignore
/// let dynamic = DynamicLimits::calculate();
/// println!("Available memory: {} MB", dynamic.available_memory_mb);
/// println!("Max result size: {} bytes", dynamic.script_limits.max_result_size);
/// ```
#[derive(Debug, Clone)]
pub struct DynamicLimits {
    /// Available system memory at calculation time (MB)
    pub available_memory_mb: f64,
    /// Calculated script limits
    pub script_limits: ScriptLimits,
}

impl DynamicLimits {
    /// Calculate limits based on current system memory.
    ///
    /// Uses the sysinfo crate to query available RAM, then calculates
    /// appropriate limits. Falls back to defaults if memory info unavailable.
    pub fn calculate() -> Self {
        let mem = crate::monitoring::get_system_memory();
        let available_mb = mem.available_mb;

        // Fallback to defaults if memory info unavailable
        if available_mb <= 0.0 {
            return Self::fallback();
        }

        // max_result_size: 5% of available RAM, clamped to [5 MB, 100 MB]
        let max_result_mb = (available_mb * 0.05).clamp(5.0, 100.0);
        let max_result_size = (max_result_mb * 1024.0 * 1024.0) as usize;

        // max_total_log_size: 0.5% of available RAM, clamped to [512 KB, 10 MB]
        let max_log_mb = (available_mb * 0.005).clamp(0.5, 10.0);
        let max_total_log_size = (max_log_mb * 1024.0 * 1024.0) as usize;

        // max_find_documents: result_size / 1KB avg doc size, clamped to [1K, 500K]
        let max_find_docs =
            (max_result_size / 1024).clamp(MIN_FIND_DOCUMENTS, DYNAMIC_MAX_FIND_DOCUMENTS);

        Self {
            available_memory_mb: available_mb,
            script_limits: ScriptLimits {
                max_operations: DEFAULT_MAX_OPERATIONS,
                timeout_ms: DEFAULT_TIMEOUT_MS,
                max_result_size,
                max_find_documents: max_find_docs,
                max_log_entry_size: MAX_SINGLE_LOG_ENTRY_BYTES,
                max_total_log_size,
            },
        }
    }

    /// Fallback limits for systems without memory info.
    fn fallback() -> Self {
        Self {
            available_memory_mb: 0.0,
            script_limits: ScriptLimits::default(),
        }
    }

    /// Check if limits should be recalculated due to memory change.
    ///
    /// Returns `true` if available memory has changed by more than 20%.
    pub fn should_recalculate(&self) -> bool {
        if self.available_memory_mb <= 0.0 {
            return false;
        }
        let current = crate::monitoring::get_system_memory();
        let change =
            (current.available_mb - self.available_memory_mb).abs() / self.available_memory_mb;
        change > 0.20
    }
}

// ============================================================
// Limits Manager (Thread-Safe Access)
// ============================================================

/// Thread-safe manager for dynamic script limits.
///
/// Provides read access to current limits and can refresh them
/// if system memory situation has changed significantly.
///
/// # Example
///
/// ```rust,ignore
/// let manager = LimitsManager::new();
///
/// // Get current limits
/// let limits = manager.get_limits();
/// engine.run_with_limits(code, params, &limits)?;
///
/// // Periodically refresh if needed
/// manager.refresh_if_needed();
/// ```
pub struct LimitsManager {
    limits: RwLock<DynamicLimits>,
}

impl LimitsManager {
    /// Create a new manager with dynamically calculated limits.
    ///
    /// Logs the initial limits at INFO level.
    pub fn new() -> Arc<Self> {
        let dynamic = DynamicLimits::calculate();
        tracing::info!(
            available_mb = format!("{:.0}", dynamic.available_memory_mb),
            max_result_mb = format!(
                "{:.1}",
                dynamic.script_limits.max_result_size as f64 / 1024.0 / 1024.0
            ),
            max_find_docs = dynamic.script_limits.max_find_documents,
            max_log_mb = format!(
                "{:.1}",
                dynamic.script_limits.max_total_log_size as f64 / 1024.0 / 1024.0
            ),
            "Dynamic script limits initialized"
        );

        Arc::new(Self {
            limits: RwLock::new(dynamic),
        })
    }

    /// Get current script limits.
    ///
    /// Returns a clone of the current limits.
    pub fn get_limits(&self) -> ScriptLimits {
        self.limits.read().script_limits.clone()
    }

    /// Get the underlying dynamic limits info.
    pub fn get_dynamic_limits(&self) -> DynamicLimits {
        self.limits.read().clone()
    }

    /// Refresh limits if memory has changed significantly (>20%).
    ///
    /// This is safe to call frequently; it only recalculates
    /// if the memory situation has actually changed.
    pub fn refresh_if_needed(&self) {
        let should_refresh = self.limits.read().should_recalculate();
        if should_refresh {
            let new_limits = DynamicLimits::calculate();
            tracing::info!(
                available_mb = format!("{:.0}", new_limits.available_memory_mb),
                max_result_mb = format!(
                    "{:.1}",
                    new_limits.script_limits.max_result_size as f64 / 1024.0 / 1024.0
                ),
                "Script limits refreshed due to memory change"
            );
            *self.limits.write() = new_limits;
        }
    }

    /// Force refresh limits regardless of memory change.
    pub fn force_refresh(&self) {
        let new_limits = DynamicLimits::calculate();
        *self.limits.write() = new_limits;
    }
}

impl Default for LimitsManager {
    fn default() -> Self {
        Self {
            limits: RwLock::new(DynamicLimits::calculate()),
        }
    }
}

// ============================================================
// Log Collector
// ============================================================

/// Collects log entries with size limits to prevent OOM.
///
/// Automatically truncates large entries and drops entries
/// when limits are exceeded.
///
/// # Example
///
/// ```rust,ignore
/// let mut collector = LogCollector::new(&limits);
/// collector.push("Log message 1");
/// collector.push("Log message 2");
/// let (logs, warning) = collector.finalize();
/// ```
pub struct LogCollector {
    entries: Vec<String>,
    total_bytes: usize,
    overflow_count: usize,
    max_entries: usize,
    max_entry_bytes: usize,
    max_total_bytes: usize,
}

impl LogCollector {
    /// Create a new log collector with the given limits.
    ///
    /// # Arguments
    ///
    /// * `limits` - Script limits configuration
    pub fn new(limits: &ScriptLimits) -> Self {
        Self {
            entries: Vec::with_capacity(1000),
            total_bytes: 0,
            overflow_count: 0,
            max_entries: MAX_LOG_ENTRIES,
            max_entry_bytes: limits.max_log_entry_size,
            max_total_bytes: limits.max_total_log_size,
        }
    }

    /// Push a log entry, respecting limits.
    ///
    /// - Truncates entries larger than `max_log_entry_size`
    /// - Drops entries if `max_entries` or `max_total_log_size` exceeded
    ///
    /// # Arguments
    ///
    /// * `s` - The log message to add
    pub fn push(&mut self, s: &str) {
        // Entry count limit
        if self.entries.len() >= self.max_entries {
            self.overflow_count += 1;
            return;
        }

        // Total bytes limit check
        if self.total_bytes >= self.max_total_bytes {
            self.overflow_count += 1;
            return;
        }

        // Truncate single entry if too large
        let entry = if s.len() > self.max_entry_bytes {
            // Find a safe UTF-8 boundary for truncation
            let truncate_at = s
                .char_indices()
                .take_while(|(i, _)| *i < self.max_entry_bytes)
                .last()
                .map(|(i, c)| i + c.len_utf8())
                .unwrap_or(0);
            format!("{}... [TRUNCATED: {} bytes]", &s[..truncate_at], s.len())
        } else {
            s.to_string()
        };

        // Check if adding this entry exceeds total bytes
        if self.total_bytes + entry.len() > self.max_total_bytes {
            self.overflow_count += 1;
            return;
        }

        self.total_bytes += entry.len();
        self.entries.push(entry);
    }

    /// Finalize and return the collected logs.
    ///
    /// Takes ownership of the entries, leaving the collector empty.
    ///
    /// # Returns
    ///
    /// A tuple of (log entries, optional overflow warning)
    pub fn finalize(&mut self) -> (Vec<String>, Option<String>) {
        let warning = if self.overflow_count > 0 {
            Some(format!(
                "[WARNING: {} log entries dropped due to limits]",
                self.overflow_count
            ))
        } else {
            None
        };
        (std::mem::take(&mut self.entries), warning)
    }

    /// Get the current number of entries.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Check if the collector is empty.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

// ============================================================
// Result Size Estimation
// ============================================================

/// Estimate the JSON size of a value without full serialization.
///
/// This is a fast approximation used to check result size limits
/// before returning the result.
///
/// # Arguments
///
/// * `value` - The JSON value to estimate
///
/// # Returns
///
/// Estimated size in bytes
pub fn estimate_json_size(value: &serde_json::Value) -> usize {
    use serde_json::Value;
    match value {
        Value::Null => 4,
        Value::Bool(true) => 4,
        Value::Bool(false) => 5,
        Value::Number(n) => n.to_string().len(),
        Value::String(s) => s.len() + 2, // quotes
        Value::Array(arr) => {
            2 + arr.iter().map(estimate_json_size).sum::<usize>() + arr.len().saturating_sub(1)
            // commas
        }
        Value::Object(obj) => {
            2 + obj
                .iter()
                .map(|(k, v)| k.len() + 3 + estimate_json_size(v)) // "key": value
                .sum::<usize>()
                + obj.len().saturating_sub(1) // commas
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_limits() {
        let limits = ScriptLimits::default();
        assert_eq!(limits.max_operations, 1_000_000);
        assert_eq!(limits.timeout_ms, 60_000);
        assert_eq!(limits.max_result_size, DEFAULT_RESULT_SIZE_BYTES);
    }

    #[test]
    fn test_effective_find_limit() {
        let limits = ScriptLimits::default();
        assert_eq!(limits.effective_find_limit(None), 10_000);
        assert_eq!(limits.effective_find_limit(Some(5_000)), 5_000);
        assert_eq!(limits.effective_find_limit(Some(2_000_000)), 1_000_000); // capped at ABSOLUTE_MAX
    }

    #[test]
    fn test_log_collector_basic() {
        let limits = ScriptLimits::default();
        let mut collector = LogCollector::new(&limits);

        collector.push("Hello");
        collector.push("World");

        let (logs, warning) = collector.finalize();
        assert_eq!(logs.len(), 2);
        assert!(warning.is_none());
    }

    #[test]
    fn test_log_collector_truncation() {
        let limits = ScriptLimits {
            max_log_entry_size: 10,
            ..Default::default()
        };

        let mut collector = LogCollector::new(&limits);
        collector.push("This is a very long message that exceeds the limit");

        let (logs, _) = collector.finalize();
        assert!(logs[0].contains("TRUNCATED"));
    }

    #[test]
    fn test_log_collector_overflow() {
        let limits = ScriptLimits {
            max_total_log_size: 20,
            ..Default::default()
        };

        let mut collector = LogCollector::new(&limits);
        collector.push("First message");
        collector.push("Second message that will overflow");

        let (logs, warning) = collector.finalize();
        assert_eq!(logs.len(), 1);
        assert!(warning.is_some());
    }

    #[test]
    fn test_estimate_json_size() {
        use serde_json::json;

        assert_eq!(estimate_json_size(&json!(null)), 4);
        assert_eq!(estimate_json_size(&json!(true)), 4);
        assert_eq!(estimate_json_size(&json!("hello")), 7); // "hello"
        assert!(estimate_json_size(&json!([1, 2, 3])) > 5);
    }

    #[test]
    fn test_dynamic_limits_calculation() {
        let limits = DynamicLimits::calculate();

        // Should have positive memory
        assert!(
            limits.available_memory_mb > 0.0,
            "Available memory should be positive"
        );

        // Result size should be within bounds
        assert!(
            limits.script_limits.max_result_size >= MIN_RESULT_SIZE_BYTES,
            "Result size should be at least MIN"
        );
        assert!(
            limits.script_limits.max_result_size <= MAX_RESULT_SIZE_BYTES,
            "Result size should not exceed MAX"
        );

        // Find docs should be within bounds
        assert!(
            limits.script_limits.max_find_documents >= MIN_FIND_DOCUMENTS,
            "Find docs should be at least MIN"
        );
        assert!(
            limits.script_limits.max_find_documents <= DYNAMIC_MAX_FIND_DOCUMENTS,
            "Find docs should not exceed DYNAMIC_MAX"
        );

        // Log size should be within bounds
        assert!(
            limits.script_limits.max_total_log_size >= MIN_TOTAL_LOG_BYTES,
            "Log size should be at least MIN"
        );
        assert!(
            limits.script_limits.max_total_log_size <= MAX_TOTAL_LOG_BYTES,
            "Log size should not exceed MAX"
        );
    }

    #[test]
    fn test_limits_manager_thread_safe() {
        let manager = LimitsManager::new();

        let limits1 = manager.get_limits();
        let limits2 = manager.get_limits();

        // Should be consistent
        assert_eq!(
            limits1.max_result_size, limits2.max_result_size,
            "Limits should be consistent across reads"
        );
    }

    #[test]
    fn test_limits_manager_refresh() {
        let manager = LimitsManager::new();

        // Refresh should not panic
        manager.refresh_if_needed();
        manager.force_refresh();

        // Limits should still be valid after refresh
        let limits = manager.get_limits();
        assert!(limits.max_result_size >= MIN_RESULT_SIZE_BYTES);
    }
}
