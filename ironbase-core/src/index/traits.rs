//! Common traits for all index types
//!
//! This module provides a unified interface for different index implementations
//! (BPlusTree, FulltextIndex, FuzzyIndex, HnswIndex) and supports lazy loading.
//!
//! # Lazy Loading Strategy (Opció A: Read-only)
//!
//! - **Startup**: Only metadata loaded if file size > threshold
//! - **Search**: `ensure_fully_loaded()` called, then search
//! - **Modify**: `ensure_fully_loaded()` called, then modify in memory
//! - **Persist**: Unchanged (full rewrite on checkpoint)
//!
//! # RAM-based Threshold
//!
//! Threshold scales with available system memory:
//!
//! | Available RAM | Lazy Threshold |
//! |---------------|----------------|
//! | < 2 GB        | 50 MB          |
//! | 4 GB          | 100 MB         |
//! | 8 GB          | 200 MB         |
//! | 16 GB         | 400 MB         |
//! | 32 GB+        | 500 MB         |

use crate::aggregation::memory_info::get_available_memory_bytes;
use crate::error::Result;

/// Calculate RAM-based lazy loading threshold
///
/// Consistent with `AggregationLimits::from_system_memory()` scaling logic.
/// Uses 2.5% of available RAM per index, clamped between 50MB and 500MB.
///
/// # Returns
///
/// Threshold in bytes. Indexes larger than this should use lazy loading.
///
/// # Example
///
/// ```rust,ignore
/// use ironbase_core::index::traits::calculate_lazy_threshold;
///
/// let threshold = calculate_lazy_threshold();
/// if index_file_size > threshold {
///     // Use lazy loading
/// }
/// ```
pub fn calculate_lazy_threshold() -> u64 {
    match get_available_memory_bytes() {
        Some(bytes) => {
            let available_mb = bytes / (1024 * 1024);
            // 2.5% available RAM per index, min 50MB, max 500MB
            let threshold_mb = (available_mb / 40).clamp(50, 500);
            threshold_mb * 1024 * 1024
        }
        None => 100 * 1024 * 1024, // Fallback: 100MB (same as fuzzy default)
    }
}

/// Common interface for all index types
///
/// This trait provides basic operations that all indexes must support.
/// It enables generic index management and memory monitoring.
pub trait IndexTrait: Send + Sync {
    /// Get index name
    fn name(&self) -> &str;

    /// Get indexed field(s)
    fn fields(&self) -> Vec<&str>;

    /// Get index size (number of entries)
    fn entry_count(&self) -> usize;

    /// Estimate memory usage in bytes
    ///
    /// This is an approximation for memory monitoring purposes.
    fn memory_usage_bytes(&self) -> usize;

    /// Check if index has disk-based storage
    fn is_disk_backed(&self) -> bool;

    /// Flush changes to disk (if applicable)
    ///
    /// For in-memory-only indexes, this is a no-op.
    fn flush(&mut self) -> Result<()>;
}

/// Extension trait for indexes that support lazy loading
///
/// Lazy-loadable indexes can defer loading data from disk until it's needed,
/// significantly reducing memory usage and startup time for large indexes.
///
/// # Implementation Pattern (follows fuzzy.rs)
///
/// ```rust,ignore
/// impl LazyLoadable for MyIndex {
///     fn is_lazy_mode(&self) -> bool {
///         self.lazy_mode && self.data.is_empty()
///     }
///
///     fn ensure_fully_loaded(&mut self) -> Result<()> {
///         if !self.lazy_mode || !self.data.is_empty() {
///             return Ok(());
///         }
///         self.data = load_from_file(&self.source_path)?;
///         self.lazy_mode = false;
///         Ok(())
///     }
///
///     fn persisted_size_bytes(&self) -> Option<u64> {
///         Some(self.data_size)
///     }
/// }
/// ```
pub trait LazyLoadable: IndexTrait {
    /// Check if lazy loading is active (data not yet in memory)
    ///
    /// Returns `true` if the index is in lazy mode AND data hasn't been loaded yet.
    fn is_lazy_mode(&self) -> bool {
        false
    }

    /// Ensure all index data is fully loaded into memory
    ///
    /// **MUST be called before any search or modify operation on lazy indexes.**
    ///
    /// This method is idempotent - calling it multiple times is safe and fast
    /// (early return if already loaded).
    ///
    /// # Errors
    ///
    /// Returns error if loading from disk fails (IO error, corrupt data, etc.)
    fn ensure_fully_loaded(&mut self) -> Result<()> {
        Ok(()) // Default: nothing to load
    }

    /// Get persisted file size in bytes (for threshold comparison)
    ///
    /// Returns `None` if index is not persisted or size is unknown.
    fn persisted_size_bytes(&self) -> Option<u64> {
        None
    }

    /// Preload frequently used data (optional optimization)
    ///
    /// Indexes may use this to warm up caches or preload hot data.
    /// Default implementation calls `ensure_fully_loaded()`.
    fn preload(&mut self) -> Result<()> {
        self.ensure_fully_loaded()
    }

    /// Evict cold data under memory pressure
    ///
    /// Returns the number of bytes freed. Indexes should evict least-recently-used
    /// data first. Default implementation returns 0 (no eviction possible).
    ///
    /// Note: For "Opció A" (read-only lazy loading), eviction is typically not
    /// supported - once loaded, data stays in memory until index is dropped.
    fn evict_cold_data(&mut self, _target_bytes: usize) -> usize {
        0
    }

    /// Get current hot data ratio (0.0 - 1.0)
    ///
    /// Indicates what percentage of the index data is currently in memory.
    /// - 1.0 = fully loaded
    /// - 0.0 = only metadata loaded (lazy mode active)
    fn hot_ratio(&self) -> f32 {
        if self.is_lazy_mode() {
            0.0
        } else {
            1.0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_calculate_lazy_threshold_returns_valid_range() {
        let threshold = calculate_lazy_threshold();

        // Should be between 50MB and 500MB
        assert!(threshold >= 50 * 1024 * 1024, "Threshold should be >= 50MB");
        assert!(threshold <= 500 * 1024 * 1024, "Threshold should be <= 500MB");
    }

    #[test]
    fn test_calculate_lazy_threshold_is_deterministic() {
        let t1 = calculate_lazy_threshold();
        let t2 = calculate_lazy_threshold();
        assert_eq!(t1, t2, "Threshold should be deterministic");
    }
}
