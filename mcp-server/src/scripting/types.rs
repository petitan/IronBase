//! Core types for script management.
//!
//! Defines structures for scripts, their metadata, versions, and execution results.

use serde::{Deserialize, Serialize};
use serde_json::Value;

// ============================================================
// Script Metadata Types
// ============================================================

/// Script metadata returned by list operations (without code).
///
/// Used for listing scripts without loading their full code content,
/// which is more efficient for displaying script catalogs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScriptInfo {
    /// Unique script name/identifier.
    pub name: String,

    /// Human-readable description of what the script does.
    pub description: Option<String>,

    /// ISO 8601 timestamp when the script was created.
    pub created_at: Option<String>,

    /// Current version number (increments on each save).
    pub version: u32,

    /// Tags for categorization and filtering.
    pub tags: Vec<String>,

    /// Number of times the script has been executed.
    pub execution_count: u64,

    /// ISO 8601 timestamp of the last execution.
    pub last_run_at: Option<String>,
}

/// Full script with code and all metadata.
///
/// Contains the complete script definition including code,
/// version history metadata, and dependencies.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Script {
    /// Unique script name/identifier.
    pub name: String,

    /// The Rhai script code.
    pub code: String,

    /// Human-readable description.
    pub description: Option<String>,

    /// ISO 8601 timestamp when the script was created.
    pub created_at: Option<String>,

    /// ISO 8601 timestamp when the script was last updated.
    pub updated_at: Option<String>,

    /// Current version number (increments on each save).
    pub version: u32,

    /// Tags for categorization.
    pub tags: Vec<String>,

    /// Dependencies on other scripts (executed before this one).
    pub dependencies: Vec<String>,
}

/// Script version history entry.
///
/// Represents a historical version of a script, stored in the
/// `_system.script_versions` collection for rollback support.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScriptVersion {
    /// Name of the script this version belongs to.
    pub script_name: String,

    /// Version number of this snapshot.
    pub version: u32,

    /// The Rhai code at this version.
    pub code: String,

    /// Description at this version.
    pub description: Option<String>,

    /// Tags at this version.
    pub tags: Vec<String>,

    /// Dependencies at this version.
    pub dependencies: Vec<String>,

    /// ISO 8601 timestamp when this version was archived.
    pub created_at: String,
}

/// Script execution statistics.
///
/// Tracks execution metrics for a script, useful for monitoring
/// and performance analysis.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScriptStats {
    /// Script name.
    pub name: String,

    /// Total number of executions.
    pub execution_count: u64,

    /// ISO 8601 timestamp of last execution.
    pub last_run_at: Option<String>,

    /// Whether the last execution succeeded.
    pub last_run_success: Option<bool>,

    /// Total execution time across all runs (milliseconds).
    pub total_execution_time_ms: u64,

    /// Average execution time (total / count).
    pub avg_execution_time_ms: f64,
}

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

/// Result of script execution.
///
/// Contains the return value, captured logs, and timing information.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScriptResult {
    /// The value returned by the script (as JSON).
    pub result: Value,

    /// Log entries captured from `print()` calls.
    pub logs: Vec<String>,

    /// Execution time in milliseconds.
    pub execution_time_ms: u64,
}

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
