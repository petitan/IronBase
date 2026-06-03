//! Timeout configuration with hierarchy: global > per-tool
//!
//! ## MCP Spec Note
//!
//! Per MCP spec (2024-11-05), timeout is CLIENT-SIDE responsibility:
//! - Client sets its own timeout
//! - On timeout, client sends `notifications/cancelled`
//! - Server handles cancellation gracefully
//!
//! However, we still implement server-side timeouts as protection against:
//! - Misbehaving clients that don't send cancellation
//! - Resource exhaustion from runaway queries
//!
//! ## Timeout Hierarchy
//!
//! ```text
//! effective_timeout = min(global_config, per_tool_default)
//! ```
//!
//! - Global default: from `config.toml` (`tool_timeout_secs`)
//! - Per-tool defaults: defined in this module based on expected operation time

use std::collections::HashMap;
use std::sync::LazyLock;
use std::time::Duration;

/// Per-tool default timeouts in milliseconds
///
/// All tools default to 5 minutes (300s) to handle large collections.
/// The global timeout in config.toml (tool_timeout_secs) can override this.
static TOOL_TIMEOUTS: LazyLock<HashMap<&'static str, u64>> = LazyLock::new(|| {
    [
        // All operations: 5 minutes (300s)
        // Large collections (78K+ docs) need this
        ("find_one", 300_000),
        ("insert_one", 300_000),
        ("delete_one", 300_000),
        ("update_one", 300_000),
        ("count", 300_000),
        ("collection_list", 300_000),
        ("collection_create", 300_000),
        ("collection_drop", 300_000),
        ("db_stats", 300_000),
        ("db_checkpoint", 300_000),
        ("index_list", 300_000),
        ("index_drop", 300_000),
        ("index_stats", 300_000),
        ("schema_get", 300_000),
        ("schema_set", 300_000),
        ("explain", 300_000),
        ("transaction_begin", 300_000),
        ("transaction_commit", 300_000),
        ("transaction_rollback", 300_000),
        ("transaction_status", 300_000),
        ("transaction_insert_one", 300_000),
        ("transaction_update_one", 300_000),
        ("transaction_delete_one", 300_000),
        ("acl_list", 300_000),
        ("acl_get", 300_000),
        ("acl_set", 300_000),
        ("acl_delete", 300_000),
        ("acl_cleanup", 300_000),
        ("listener_list", 300_000),
        ("listener_get", 300_000),
        ("listener_create", 300_000),
        ("listener_delete", 300_000),
        ("listener_enable", 300_000),
        ("listener_disable", 300_000),
        ("script_list", 300_000),
        ("script_get", 300_000),
        ("script_save", 300_000),
        ("script_delete", 300_000),
        ("script_history", 300_000),
        ("script_rollback", 300_000),
        ("script_version_get", 300_000),
        ("script_tags_add", 300_000),
        ("script_tags_remove", 300_000),
        ("script_stats", 300_000),
        ("find", 300_000),
        ("find_with_hint", 300_000),
        ("insert_many", 300_000),
        ("update_many", 300_000),
        ("delete_many", 300_000),
        ("distinct", 300_000),
        ("fulltext_search", 300_000),
        ("fuzzy_search", 300_000),
        ("aggregate", 300_000),
        ("index_create", 300_000),
        ("script_exec", 300_000),
        ("script_run", 300_000),
        ("db_compact", 300_000),
        ("db_open", 300_000),
    ]
    .into_iter()
    .collect()
});

/// Get the default timeout for a specific tool
///
/// Returns `None` if the tool has no specific default (use global).
pub fn get_tool_timeout(tool_name: &str) -> Option<Duration> {
    TOOL_TIMEOUTS
        .get(tool_name)
        .map(|&ms| Duration::from_millis(ms))
}

/// Calculate effective timeout for a tool
///
/// Returns the minimum of:
/// 1. Global timeout from config
/// 2. Per-tool default (if defined)
///
/// ## Arguments
///
/// * `global_ms` - Global timeout from config.toml in milliseconds
/// * `tool_name` - Name of the tool being executed
///
/// ## Example
///
/// ```ignore
/// let global = config.tool_timeout_secs * 1000;
/// let timeout = effective_timeout(global, "find");
/// // If global=60000 and find=30000, returns 30s
/// ```
pub fn effective_timeout(global_ms: u64, tool_name: &str) -> Duration {
    let tool_default = TOOL_TIMEOUTS.get(tool_name).copied().unwrap_or(global_ms);
    Duration::from_millis(global_ms.min(tool_default))
}

/// Tools that should run without timeout
///
/// Index creation operations can take a long time on large collections
/// (e.g., 78K+ documents) and should not be interrupted by server-side timeouts.
/// Clients can still cancel via the MCP protocol if needed.
static NO_TIMEOUT_TOOLS: &[&str] = &[
    // index_create is now generic across all subtypes (btree/fulltext/fuzzy/vector),
    // each of which can take a long time on large collections.
    "index_create",
    "db_compact",
    "script_run",
    "script_exec",
];

/// Check if a tool should run without timeout
pub fn is_no_timeout_tool(tool_name: &str) -> bool {
    NO_TIMEOUT_TOOLS.contains(&tool_name)
}

/// Get effective timeout for a tool, or None if it should run without timeout
///
/// Returns:
/// - `None` for index creation operations (they can take arbitrarily long)
/// - `Some(Duration)` for all other operations
///
/// ## Arguments
///
/// * `global_ms` - Global timeout from config.toml in milliseconds
/// * `tool_name` - Name of the tool being executed
///
/// ## Example
///
/// ```ignore
/// let global = config.tool_timeout_secs * 1000;
/// match effective_timeout_option(global, "index_create") {
///     None => { /* No deadline - index creation can take as long as needed */ }
///     Some(timeout) => { /* Set deadline for regular operations */ }
/// }
/// ```
pub fn effective_timeout_option(global_ms: u64, tool_name: &str) -> Option<Duration> {
    if is_no_timeout_tool(tool_name) {
        None // Index operations run without timeout
    } else {
        Some(effective_timeout(global_ms, tool_name))
    }
}

/// Get all tool timeouts (for debugging/documentation)
pub fn all_tool_timeouts() -> &'static HashMap<&'static str, u64> {
    &TOOL_TIMEOUTS
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_effective_timeout_uses_tool_default() {
        // find has 300s default, global is 60s → uses 60s (min)
        let timeout = effective_timeout(60_000, "find");
        assert_eq!(timeout, Duration::from_millis(60_000));
    }

    #[test]
    fn test_effective_timeout_uses_global_if_smaller() {
        // Global 10s is smaller than find's 30s
        let timeout = effective_timeout(10_000, "find");
        assert_eq!(timeout, Duration::from_millis(10_000));
    }

    #[test]
    fn test_effective_timeout_unknown_tool_uses_global() {
        // Unknown tool uses global
        let timeout = effective_timeout(45_000, "unknown_tool");
        assert_eq!(timeout, Duration::from_millis(45_000));
    }

    #[test]
    fn test_get_tool_timeout() {
        assert_eq!(
            get_tool_timeout("find"),
            Some(Duration::from_millis(300_000))
        );
        assert_eq!(
            get_tool_timeout("db_compact"),
            Some(Duration::from_millis(300_000))
        );
        assert_eq!(get_tool_timeout("unknown"), None);
    }

    #[test]
    fn test_all_tools_have_reasonable_timeouts() {
        for (tool, &ms) in all_tool_timeouts().iter() {
            // All timeouts should be at least 1 second
            assert!(ms >= 1000, "{} has timeout < 1s", tool);
            // All timeouts should be at most 5 minutes
            assert!(ms <= 300_000, "{} has timeout > 5min", tool);
        }
    }
}
