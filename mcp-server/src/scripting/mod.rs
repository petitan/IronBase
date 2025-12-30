//! Script management and execution for IronBase MCP Server.
//!
//! This module provides:
//! - CRUD operations for scripts stored in `_system.scripts`
//! - Rhai script execution engine with database bindings
//! - Version history and rollback support
//! - Resource limits to prevent OOM and DoS attacks
//! - Graceful shutdown support for running scripts
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────┐
//! │                    ScriptManager                             │
//! │  (CRUD, versioning, tags, dependencies, execution stats)    │
//! └─────────────────────────────────────────────────────────────┘
//!                              │
//!                              ▼
//! ┌─────────────────────────────────────────────────────────────┐
//! │                      RhaiEngine                              │
//! │  (Script execution with limits, cancellation, timeout)      │
//! └─────────────────────────────────────────────────────────────┘
//!                              │
//!              ┌───────────────┴───────────────┐
//!              ▼                               ▼
//! ┌────────────────────────┐     ┌────────────────────────────┐
//! │    db_functions.rs     │     │  utility_functions.rs      │
//! │  (db_find, db_insert,  │     │  (base64, uuid, timestamp) │
//! │   db_update, ...)      │     │                            │
//! └────────────────────────┘     └────────────────────────────┘
//! ```
//!
//! # Resource Limits
//!
//! All scripts are executed with resource limits to prevent abuse:
//! - **Operation limit**: Max 1,000,000 Rhai operations
//! - **Timeout**: Default 60 seconds
//! - **Result size**: Max 10 MB
//! - **Log size**: Max 1 MB total, 64 KB per entry
//! - **Query limit**: Max 10,000 documents per db_find()
//!
//! # Example
//!
//! ```rust,ignore
//! use mcp_server::scripting::{ScriptManager, RhaiEngine, ScriptLimits};
//!
//! // Create manager and engine
//! let manager = ScriptManager::new(adapter.clone());
//! let engine = RhaiEngine::new(adapter);
//!
//! // Save a script
//! manager.save("greet", r#"
//!     let name = params.name;
//!     "Hello, " + name + "!"
//! "#, Some("Greeting script"), None, None)?;
//!
//! // Run the script
//! let result = manager.run_script("greet", Some(json!({"name": "World"})), &engine)?;
//! println!("{}", result.result); // "Hello, World!"
//!
//! // Run with custom limits
//! let limits = ScriptLimits::with_timeout(5000); // 5 seconds
//! let result = manager.run_script_with_limits("greet", None, &engine, limits)?;
//! ```
//!
//! # Security
//!
//! - Scripts cannot write to `_system.*` collections
//! - All queries are limited to prevent OOM
//! - Operation count prevents infinite loops
//! - Timeout prevents long-running scripts
//! - Result size limits prevent memory exhaustion

// Internal modules
mod conversion;
mod db_functions;
mod engine;
mod error;
mod limits;
mod manager;
mod tracker;
mod types;
mod utility_functions;

// Re-export public API
pub use conversion::{dynamic_to_json, json_to_dynamic, map_to_json};
pub use engine::RhaiEngine;
pub use error::format_rhai_error;
pub use limits::{
    estimate_json_size, LogCollector, ScriptLimits,
    // Constants
    ABSOLUTE_MAX_FIND_DOCUMENTS,
    DEFAULT_MAX_OPERATIONS,
    DEFAULT_TIMEOUT_MS,
    MAX_FIND_DOCUMENTS,
    MAX_LOG_ENTRIES,
    MAX_RESULT_SIZE_BYTES,
    MAX_SINGLE_LOG_ENTRY_BYTES,
    MAX_TOTAL_LOG_BYTES,
};
pub use manager::ScriptManager;
pub use tracker::{ActiveScriptTracker, ScriptExecutionGuard};
pub use types::{
    Script, ScriptInfo, ScriptListFilter, ScriptOptions, ScriptResult, ScriptStats, ScriptVersion,
};

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::Arc;
    use tempfile::TempDir;

    fn create_test_adapter() -> (Arc<crate::adapter::IronBaseAdapter>, TempDir) {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.mlite");
        let adapter = Arc::new(crate::adapter::IronBaseAdapter::new(&db_path).unwrap());
        (adapter, temp_dir)
    }

    #[test]
    fn test_script_save_and_get() {
        let (adapter, _temp) = create_test_adapter();
        let manager = ScriptManager::new(adapter);

        let version = manager
            .save(
                "test_script",
                "let x = 1 + 1;",
                Some("Test script"),
                None,
                None,
            )
            .unwrap();
        assert_eq!(version, 1);

        let script = manager.get("test_script").unwrap().unwrap();
        assert_eq!(script.name, "test_script");
        assert_eq!(script.code, "let x = 1 + 1;");
        assert_eq!(script.description, Some("Test script".to_string()));
        assert_eq!(script.version, 1);
    }

    #[test]
    fn test_script_list() {
        let (adapter, _temp) = create_test_adapter();
        let manager = ScriptManager::new(adapter);

        manager
            .save("script1", "code1", Some("First"), None, None)
            .unwrap();
        manager
            .save("script2", "code2", Some("Second"), None, None)
            .unwrap();

        let scripts = manager.list(None).unwrap();
        assert_eq!(scripts.len(), 2);

        let names: Vec<&str> = scripts.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"script1"));
        assert!(names.contains(&"script2"));
    }

    #[test]
    fn test_script_update() {
        let (adapter, _temp) = create_test_adapter();
        let manager = ScriptManager::new(adapter);

        let v1 = manager
            .save("updatable", "v1", Some("Version 1"), None, None)
            .unwrap();
        assert_eq!(v1, 1);

        let v2 = manager
            .save("updatable", "v2", Some("Version 2"), None, None)
            .unwrap();
        assert_eq!(v2, 2);

        let script = manager.get("updatable").unwrap().unwrap();
        assert_eq!(script.code, "v2");
        assert_eq!(script.version, 2);

        let scripts = manager.list(None).unwrap();
        assert_eq!(scripts.len(), 1);
    }

    #[test]
    fn test_script_delete() {
        let (adapter, _temp) = create_test_adapter();
        let manager = ScriptManager::new(adapter);

        manager.save("deletable", "code", None, None, None).unwrap();
        let deleted = manager.delete("deletable").unwrap();
        assert!(deleted);

        let script = manager.get("deletable").unwrap();
        assert!(script.is_none());

        let deleted = manager.delete("nonexistent").unwrap();
        assert!(!deleted);
    }

    #[test]
    fn test_rhai_simple_arithmetic() {
        let (adapter, _temp) = create_test_adapter();
        let engine = RhaiEngine::new(adapter);

        let result = engine.run("1 + 2", None).unwrap();
        assert_eq!(result.result, json!(3));
    }

    #[test]
    fn test_rhai_with_params() {
        let (adapter, _temp) = create_test_adapter();
        let engine = RhaiEngine::new(adapter);

        let result = engine
            .run("params.x + params.y", Some(json!({"x": 10, "y": 5})))
            .unwrap();
        assert_eq!(result.result, json!(15));
    }

    #[test]
    fn test_rhai_db_insert_and_find() {
        let (adapter, _temp) = create_test_adapter();
        let engine = RhaiEngine::new(adapter);

        let result = engine
            .run(
                r#"
            let doc = #{ name: "Alice", age: 30 };
            let id = db_insert_one("users", doc);
            let found = db_find("users", #{});
            found.len()
        "#,
                None,
            )
            .unwrap();

        assert_eq!(result.result, json!(1));
    }

    #[test]
    fn test_rhai_db_find_with_limit() {
        let (adapter, _temp) = create_test_adapter();
        let engine = RhaiEngine::new(adapter);

        let result = engine
            .run(
                r#"
            for i in 0..10 {
                db_insert_one("items", #{ idx: i });
            }
            let found = db_find("items", #{}, #{ limit: 3 });
            found.len()
        "#,
                None,
            )
            .unwrap();

        assert_eq!(result.result, json!(3));
    }

    #[test]
    fn test_script_limits() {
        let limits = ScriptLimits::default();
        assert_eq!(limits.max_operations, 1_000_000);
        assert_eq!(limits.timeout_ms, 60_000);
        assert_eq!(limits.max_result_size, 10 * 1024 * 1024);
    }

    #[test]
    fn test_tracker_basic() {
        let tracker = Arc::new(ActiveScriptTracker::new());
        assert!(tracker.can_accept());
        assert_eq!(tracker.active_count(), 0);

        {
            let _guard = tracker.track().unwrap();
            assert_eq!(tracker.active_count(), 1);
        }
        assert_eq!(tracker.active_count(), 0);
    }

    #[test]
    fn test_tracker_shutdown() {
        let tracker = Arc::new(ActiveScriptTracker::new());

        let _guard = tracker.track().unwrap();
        tracker.signal_shutdown();

        assert!(!tracker.can_accept());
        assert!(tracker.track().is_none());
        assert_eq!(tracker.active_count(), 1);
    }
}
