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
/// // Unsafe mode - manual checkpoint only
/// let mode = DurabilityMode::unsafe_manual();
///
/// // Unsafe mode - auto checkpoint every 10000 operations
/// let mode = DurabilityMode::unsafe_auto(10000);
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

    /// Unsafe mode: No auto-commit, optional auto-checkpoint
    /// - No WAL for normal operations
    /// - Fast but data loss on crash
    /// - User can set auto_checkpoint_ops for periodic checkpoint
    /// - If None, user must explicitly call checkpoint()
    Unsafe {
        /// Optional: automatically checkpoint after N operations
        /// None = manual checkpoint only (original behavior)
        /// Some(n) = checkpoint every n operations
        auto_checkpoint_ops: Option<usize>,
    },
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
            DurabilityMode::Unsafe { .. } => false,
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

    /// Get auto checkpoint ops if in unsafe mode with auto checkpoint
    pub fn auto_checkpoint_ops(&self) -> Option<usize> {
        match self {
            DurabilityMode::Unsafe {
                auto_checkpoint_ops,
            } => *auto_checkpoint_ops,
            _ => None,
        }
    }

    /// Create Unsafe mode without auto checkpoint (original behavior)
    pub fn unsafe_manual() -> Self {
        DurabilityMode::Unsafe {
            auto_checkpoint_ops: None,
        }
    }

    /// Create Unsafe mode with auto checkpoint every N operations
    pub fn unsafe_auto(checkpoint_every: usize) -> Self {
        DurabilityMode::Unsafe {
            auto_checkpoint_ops: Some(checkpoint_every),
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
        assert!(!DurabilityMode::unsafe_manual().is_auto_commit());
        assert!(!DurabilityMode::unsafe_auto(1000).is_auto_commit());
    }

    #[test]
    fn test_is_safe() {
        assert!(DurabilityMode::Safe.is_safe());
        assert!(!DurabilityMode::Batch { batch_size: 100 }.is_safe());
        assert!(!DurabilityMode::unsafe_manual().is_safe());
    }

    #[test]
    fn test_batch_size() {
        assert_eq!(DurabilityMode::Safe.batch_size(), None);
        assert_eq!(
            DurabilityMode::Batch { batch_size: 100 }.batch_size(),
            Some(100)
        );
        assert_eq!(DurabilityMode::unsafe_manual().batch_size(), None);
    }

    #[test]
    fn test_auto_checkpoint_ops() {
        assert_eq!(DurabilityMode::Safe.auto_checkpoint_ops(), None);
        assert_eq!(
            DurabilityMode::Batch { batch_size: 100 }.auto_checkpoint_ops(),
            None
        );
        assert_eq!(DurabilityMode::unsafe_manual().auto_checkpoint_ops(), None);
        assert_eq!(
            DurabilityMode::unsafe_auto(5000).auto_checkpoint_ops(),
            Some(5000)
        );
    }

    #[test]
    fn test_unsafe_constructors() {
        let manual = DurabilityMode::unsafe_manual();
        assert!(matches!(
            manual,
            DurabilityMode::Unsafe {
                auto_checkpoint_ops: None
            }
        ));

        let auto = DurabilityMode::unsafe_auto(10000);
        assert!(matches!(
            auto,
            DurabilityMode::Unsafe {
                auto_checkpoint_ops: Some(10000)
            }
        ));
    }
}
