//! Core types for script management.
//!
//! Defines structures for scripts, their metadata, versions, and execution results.

use serde::{Deserialize, Serialize};

// ============================================================
// Script Metadata Types
// ============================================================

// Script metadata, versions, statistics: shared with clients, re-exported under
// their old path.
pub use ironbase_mcp_types::responses::{Script, ScriptInfo, ScriptStats, ScriptVersion};

// ============================================================
// Script Filtering
// ============================================================

/// Filter options for script listing.
///
/// Allows filtering scripts by tags with AND/OR logic.
#[derive(Debug, Clone, Default)]
pub struct ScriptListFilter {
    /// Filter by these tags.
    pub tags: Option<Vec<String>>,

    /// If true, require ALL tags (AND logic).
    /// If false, require ANY tag (OR logic).
    pub match_all_tags: bool,
}

// ============================================================
// Execution Results
// ============================================================

// Execution result: shared with clients, re-exported under its old path.
pub use ironbase_mcp_types::responses::ScriptResult;

// ============================================================
// Legacy Compatibility
// ============================================================

/// Options for script execution.
///
/// Maintained for backward compatibility. Prefer using `ScriptLimits`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScriptOptions {
    /// Maximum number of operations (DoS protection).
    ///
    /// Default: 1,000,000. Set to 0 to use default (not unlimited).
    #[serde(default = "default_max_operations")]
    pub max_operations: u64,
}

fn default_max_operations() -> u64 {
    super::limits::DEFAULT_MAX_OPERATIONS
}

impl Default for ScriptOptions {
    fn default() -> Self {
        Self {
            max_operations: default_max_operations(),
        }
    }
}

impl ScriptOptions {
    /// Create options with custom max operations limit.
    ///
    /// # Arguments
    ///
    /// * `max_ops` - Maximum number of Rhai operations
    pub fn with_max_operations(max_ops: u64) -> Self {
        Self {
            max_operations: max_ops,
        }
    }
}

impl From<ScriptOptions> for super::limits::ScriptLimits {
    fn from(opts: ScriptOptions) -> Self {
        super::limits::ScriptLimits {
            max_operations: opts.max_operations,
            ..Default::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_script_options_default() {
        let opts = ScriptOptions::default();
        assert_eq!(opts.max_operations, 1_000_000);
    }

    #[test]
    fn test_script_list_filter_default() {
        let filter = ScriptListFilter::default();
        assert!(filter.tags.is_none());
        assert!(!filter.match_all_tags);
    }
}
