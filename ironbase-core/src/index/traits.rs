//! Common traits for all index types
//!
//! This module provides a unified interface for different index implementations
//! (BPlusTree, FulltextIndex, FuzzyIndex) and supports lazy loading capabilities.

use crate::error::Result;

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
/// significantly reducing memory usage for large indexes.
pub trait LazyLoadable: IndexTrait {
    /// Preload frequently used data (optional optimization)
    ///
    /// Indexes may use this to warm up caches or preload hot data.
    /// Default implementation is a no-op.
    fn preload(&mut self) -> Result<()> {
        Ok(())
    }

    /// Evict cold data under memory pressure
    ///
    /// Returns the number of bytes freed. Indexes should evict least-recently-used
    /// data first. Default implementation returns 0 (no eviction possible).
    fn evict_cold_data(&mut self, _target_bytes: usize) -> usize {
        0
    }

    /// Get current hot data ratio (0.0 - 1.0)
    ///
    /// Indicates what percentage of the index data is currently in memory.
    /// 1.0 means fully loaded, 0.0 means only metadata is loaded.
    fn hot_ratio(&self) -> f32 {
        1.0
    }

    /// Check if lazy loading is active
    fn is_lazy_mode(&self) -> bool {
        false
    }
}
