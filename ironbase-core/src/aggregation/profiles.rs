// src/aggregation/profiles.rs
// Memory profile tiers and unified scaling for aggregation limits
//
// DESIGN (2026-01):
// Provides 5 memory tiers (Tiny/Small/Medium/Large/XLarge) with a unified
// scaling table. No upper caps - the XLarge tier values are used for all
// systems with > 32GB available RAM.
//
// The scaling table is designed to prevent OOM while allowing reasonable
// throughput for each memory tier.

use crate::aggregation::memory_info::get_available_memory_bytes;
use crate::aggregation::types::AggregationLimits;

/// Helper: get available memory in MB
fn get_available_memory_mb() -> u64 {
    get_available_memory_bytes()
        .map(|bytes| bytes / (1024 * 1024))
        .unwrap_or(256) // Default to 256MB if detection fails
}

/// Memory tier based on available system RAM
///
/// Each tier corresponds to a specific set of aggregation limits that
/// balance throughput with OOM safety.
#[allow(dead_code)] // Phase 2: will be used in pipeline.rs
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MemoryTier {
    /// < 512 MB available RAM
    Tiny,
    /// 512 MB - 2 GB available RAM
    Small,
    /// 2 GB - 8 GB available RAM
    Medium,
    /// 8 GB - 32 GB available RAM
    Large,
    /// > 32 GB available RAM
    XLarge,
}

impl MemoryTier {
    /// Detect memory tier from available megabytes
    pub fn from_available_mb(mb: u64) -> Self {
        match mb {
            0..=511 => Self::Tiny,
            512..=2047 => Self::Small,
            2048..=8191 => Self::Medium,
            8192..=32767 => Self::Large,
            _ => Self::XLarge,
        }
    }

    /// Detect memory tier from current system state
    pub fn from_system() -> Self {
        let available_mb = get_available_memory_mb();
        Self::from_available_mb(available_mb)
    }

    /// Get the scaling entry for this tier
    ///
    /// FIX (2026-01-25): Added global accumulator limits
    /// Global limits are ~100x per-group limits to allow many groups
    fn scaling_entry(&self) -> ScalingEntry {
        match self {
            Self::Tiny => ScalingEntry {
                max_docs_without_match: 10_000,
                max_docs_with_match: 50_000,
                max_group_count: 10_000, // FIX: was 5K, streaming only uses 640KB
                max_push_elements: 10_000,
                max_addtoset_elements: 10_000,
                max_total_push_elements: 100_000,
                max_total_addtoset_elements: 50_000,
                max_unwind_output: 100_000,
                max_memory_mb: 64,
            },
            Self::Small => ScalingEntry {
                max_docs_without_match: 50_000,
                max_docs_with_match: 250_000,
                max_group_count: 50_000, // FIX: was 25K
                max_push_elements: 50_000,
                max_addtoset_elements: 50_000,
                max_total_push_elements: 500_000,
                max_total_addtoset_elements: 250_000,
                max_unwind_output: 500_000,
                max_memory_mb: 128,
            },
            Self::Medium => ScalingEntry {
                max_docs_without_match: 100_000,
                max_docs_with_match: 500_000,
                max_group_count: 100_000, // FIX: was 50K
                max_push_elements: 100_000,
                max_addtoset_elements: 100_000,
                max_total_push_elements: 1_000_000,
                max_total_addtoset_elements: 500_000,
                max_unwind_output: 1_000_000,
                max_memory_mb: 256,
            },
            Self::Large => ScalingEntry {
                max_docs_without_match: 250_000,
                max_docs_with_match: 1_000_000,
                max_group_count: 200_000, // FIX: was 100K
                max_push_elements: 250_000,
                max_addtoset_elements: 250_000,
                max_total_push_elements: 2_500_000,
                max_total_addtoset_elements: 1_250_000,
                max_unwind_output: 2_500_000,
                max_memory_mb: 512,
            },
            Self::XLarge => ScalingEntry {
                max_docs_without_match: 500_000,
                max_docs_with_match: 2_000_000,
                max_group_count: 500_000, // FIX: was 250K
                max_push_elements: 500_000,
                max_addtoset_elements: 500_000,
                max_total_push_elements: 5_000_000,
                max_total_addtoset_elements: 2_500_000,
                max_unwind_output: 5_000_000,
                max_memory_mb: 1024,
            },
        }
    }
}

/// Internal scaling entry for a memory tier
///
/// FIX (2026-01-25): Added global accumulator limits
#[derive(Debug, Clone, Copy)]
struct ScalingEntry {
    max_docs_without_match: usize,
    max_docs_with_match: usize,
    max_group_count: usize,
    max_push_elements: usize,
    max_addtoset_elements: usize,
    max_total_push_elements: usize,
    max_total_addtoset_elements: usize,
    max_unwind_output: usize,
    max_memory_mb: usize,
}

impl From<ScalingEntry> for AggregationLimits {
    fn from(entry: ScalingEntry) -> Self {
        Self {
            max_docs_without_match: entry.max_docs_without_match,
            max_docs_with_match: entry.max_docs_with_match,
            max_group_count: entry.max_group_count,
            max_push_elements: entry.max_push_elements,
            max_addtoset_elements: entry.max_addtoset_elements,
            max_total_push_elements: entry.max_total_push_elements,
            max_total_addtoset_elements: entry.max_total_addtoset_elements,
            max_unwind_output: entry.max_unwind_output,
            max_memory_mb: entry.max_memory_mb,
        }
    }
}

// ========== Helper functions for AggregationLimits ==========

/// Create AggregationLimits from a memory tier
///
/// This is the core function used by from_system_memory(), with_memory_budget(), etc.
#[allow(dead_code)] // Phase 2: will be used in pipeline.rs
pub fn limits_from_tier(tier: MemoryTier) -> AggregationLimits {
    tier.scaling_entry().into()
}

/// Create AggregationLimits from available memory in MB
#[allow(dead_code)] // Phase 2: will be used in pipeline.rs
pub fn limits_from_available_mb(mb: u64) -> AggregationLimits {
    limits_from_tier(MemoryTier::from_available_mb(mb))
}

/// Create AggregationLimits based on current system RAM
#[allow(dead_code)] // Phase 2: will be used in pipeline.rs
pub fn limits_from_system() -> AggregationLimits {
    limits_from_tier(MemoryTier::from_system())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_memory_tier_detection() {
        assert_eq!(MemoryTier::from_available_mb(0), MemoryTier::Tiny);
        assert_eq!(MemoryTier::from_available_mb(256), MemoryTier::Tiny);
        assert_eq!(MemoryTier::from_available_mb(511), MemoryTier::Tiny);
        assert_eq!(MemoryTier::from_available_mb(512), MemoryTier::Small);
        assert_eq!(MemoryTier::from_available_mb(1024), MemoryTier::Small);
        assert_eq!(MemoryTier::from_available_mb(2048), MemoryTier::Medium);
        assert_eq!(MemoryTier::from_available_mb(4096), MemoryTier::Medium);
        assert_eq!(MemoryTier::from_available_mb(8192), MemoryTier::Large);
        assert_eq!(MemoryTier::from_available_mb(16384), MemoryTier::Large);
        assert_eq!(MemoryTier::from_available_mb(32768), MemoryTier::XLarge);
        assert_eq!(MemoryTier::from_available_mb(65536), MemoryTier::XLarge);
    }

    #[test]
    fn test_tier_limits_scaling() {
        let tiny = limits_from_tier(MemoryTier::Tiny);
        let small = limits_from_tier(MemoryTier::Small);
        let medium = limits_from_tier(MemoryTier::Medium);
        let large = limits_from_tier(MemoryTier::Large);
        let xlarge = limits_from_tier(MemoryTier::XLarge);

        // Each tier should have progressively higher limits
        assert!(tiny.max_docs_without_match < small.max_docs_without_match);
        assert!(small.max_docs_without_match < medium.max_docs_without_match);
        assert!(medium.max_docs_without_match < large.max_docs_without_match);
        assert!(large.max_docs_without_match < xlarge.max_docs_without_match);

        assert!(tiny.max_group_count < small.max_group_count);
        assert!(small.max_group_count < medium.max_group_count);
        assert!(medium.max_group_count < large.max_group_count);
        assert!(large.max_group_count < xlarge.max_group_count);

        assert!(tiny.max_memory_mb < small.max_memory_mb);
        assert!(small.max_memory_mb < medium.max_memory_mb);
        assert!(medium.max_memory_mb < large.max_memory_mb);
        assert!(large.max_memory_mb < xlarge.max_memory_mb);
    }

    #[test]
    fn test_limits_from_available_mb() {
        let tiny = limits_from_available_mb(100);
        let small = limits_from_available_mb(1024);
        let medium = limits_from_available_mb(4096);
        let large = limits_from_available_mb(16384);
        let xlarge = limits_from_available_mb(65536);

        assert_eq!(tiny.max_docs_without_match, 10_000); // Tiny tier
        assert_eq!(small.max_docs_without_match, 50_000); // Small tier
        assert_eq!(medium.max_docs_without_match, 100_000); // Medium tier
        assert_eq!(large.max_docs_without_match, 250_000); // Large tier
        assert_eq!(xlarge.max_docs_without_match, 500_000); // XLarge tier
    }

    #[test]
    fn test_tiny_tier_values() {
        let tiny = limits_from_tier(MemoryTier::Tiny);

        assert_eq!(tiny.max_docs_without_match, 10_000);
        assert_eq!(tiny.max_docs_with_match, 50_000);
        // FIX (2026-01-25): was 5K, now 10K - streaming $group uses only 640KB
        assert_eq!(tiny.max_group_count, 10_000);
        assert_eq!(tiny.max_push_elements, 10_000);
        assert_eq!(tiny.max_addtoset_elements, 10_000);
        // FIX (2026-01-25): new global limits
        assert_eq!(tiny.max_total_push_elements, 100_000);
        assert_eq!(tiny.max_total_addtoset_elements, 50_000);
        assert_eq!(tiny.max_unwind_output, 100_000);
        assert_eq!(tiny.max_memory_mb, 64);
    }

    #[test]
    fn test_xlarge_tier_values() {
        let xlarge = limits_from_tier(MemoryTier::XLarge);

        assert_eq!(xlarge.max_docs_without_match, 500_000);
        assert_eq!(xlarge.max_docs_with_match, 2_000_000);
        // FIX (2026-01-25): was 250K, now 500K
        assert_eq!(xlarge.max_group_count, 500_000);
        assert_eq!(xlarge.max_push_elements, 500_000);
        assert_eq!(xlarge.max_addtoset_elements, 500_000);
        // FIX (2026-01-25): new global limits
        assert_eq!(xlarge.max_total_push_elements, 5_000_000);
        assert_eq!(xlarge.max_total_addtoset_elements, 2_500_000);
        assert_eq!(xlarge.max_unwind_output, 5_000_000);
        assert_eq!(xlarge.max_memory_mb, 1024);
    }

    #[test]
    fn test_limits_from_system_returns_valid_limits() {
        // Just verify it doesn't panic and returns valid limits
        let limits = limits_from_system();

        assert!(limits.max_docs_without_match > 0);
        assert!(limits.max_docs_with_match >= limits.max_docs_without_match);
        assert!(limits.max_group_count > 0);
        assert!(limits.max_push_elements > 0);
        assert!(limits.max_addtoset_elements > 0);
        assert!(limits.max_unwind_output > 0);
        assert!(limits.max_memory_mb > 0);
    }
}
