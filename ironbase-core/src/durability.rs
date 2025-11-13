//! Durability modes for auto-commit behavior
//!
//! This module defines how operations are protected against data loss.
//! Similar to SQL databases, IronBase can operate in different durability modes.

use serde::{Deserialize, Serialize};

/// Durability mode determines how operations are committed to WAL
///
/// # Modes
///
/// - **Safe**: Every operation is auto-committed (like SQL auto-commit)
///   - WAL written for every operation
///   - fsync after every commit
///   - Slow but guaranteed durability
///   - Performance: ~1,000-5,000 inserts/sec
///
/// - **Batch**: Operations batched, periodic auto-commit
///   - WAL written every N operations
///   - Bounded data loss (max N operations)
///   - Good balance of safety and performance
///   - Performance: ~20,000-50,000 inserts/sec
///
/// - **Unsafe**: No auto-commit, manual checkpoint required
///   - No WAL for normal operations
///   - Fast but data loss on crash
///   - User must explicitly call checkpoint()
///   - Performance: ~50,000-100,000 inserts/sec
///
/// # Examples
///
/// ```rust
/// use ironbase_core::DurabilityMode;
///
/// // Safe mode (default)
/// let mode = DurabilityMode::Safe;
///
/// // Batch mode with 100 operations per commit
/// let mode = DurabilityMode::Batch { batch_size: 100 };
///
/// // Unsafe mode (opt-in for performance)
/// let mode = DurabilityMode::Unsafe;
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DurabilityMode {
    /// Safe mode: Every operation is auto-committed (like SQL)
    /// - WAL written for every operation
    /// - fsync after every commit
    /// - Slow but guaranteed durability
    Safe,

    /// Batch mode: Operations batched, periodic auto-commit
    /// - WAL written every N operations
    /// - Bounded data loss (max N operations)
    /// - Good balance of safety and performance
    Batch {
        /// Number of operations before auto-commit
        batch_size: usize,
    },

    /// Unsafe mode: No auto-commit, manual checkpoint required
    /// - No WAL for normal operations
    /// - Fast but data loss on crash
    /// - User must explicitly call checkpoint()
    Unsafe,
}

impl Default for DurabilityMode {
    /// Default to Safe mode (like SQL databases)
    fn default() -> Self {
        DurabilityMode::Safe
    }
}

impl DurabilityMode {
    /// Check if this mode requires auto-commit
    pub fn is_auto_commit(&self) -> bool {
        match self {
            DurabilityMode::Safe => true,
            DurabilityMode::Batch { .. } => true,
            DurabilityMode::Unsafe => false,
        }
    }

    /// Check if this mode is safe (zero data loss guarantee)
    pub fn is_safe(&self) -> bool {
        matches!(self, DurabilityMode::Safe)
    }

    /// Get batch size if in batch mode
    pub fn batch_size(&self) -> Option<usize> {
        match self {
            DurabilityMode::Batch { batch_size } => Some(*batch_size),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_is_safe() {
        assert_eq!(DurabilityMode::default(), DurabilityMode::Safe);
    }

    #[test]
    fn test_is_auto_commit() {
        assert!(DurabilityMode::Safe.is_auto_commit());
        assert!(DurabilityMode::Batch { batch_size: 100 }.is_auto_commit());
        assert!(!DurabilityMode::Unsafe.is_auto_commit());
    }

    #[test]
    fn test_is_safe() {
        assert!(DurabilityMode::Safe.is_safe());
        assert!(!DurabilityMode::Batch { batch_size: 100 }.is_safe());
        assert!(!DurabilityMode::Unsafe.is_safe());
    }

    #[test]
    fn test_batch_size() {
        assert_eq!(DurabilityMode::Safe.batch_size(), None);
        assert_eq!(
            DurabilityMode::Batch { batch_size: 100 }.batch_size(),
            Some(100)
        );
        assert_eq!(DurabilityMode::Unsafe.batch_size(), None);
    }
}
