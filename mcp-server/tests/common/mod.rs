//! Shared integration-test helpers.
//!
//! One copy of the adapter/dispatch plumbing for every `tests/*.rs` binary,
//! instead of a per-file duplicate of the verbose 9-arg `dispatch_tool` call.
#![allow(dead_code)] // not every test binary uses every helper

use mcp_ironbase::{dispatch_tool, IronBaseAdapter, McpError};
use serde_json::Value;
use std::sync::Arc;
use tempfile::TempDir;

/// Create a test adapter backed by a temporary database.
pub fn create_test_adapter() -> (Arc<IronBaseAdapter>, TempDir) {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let db_path = temp_dir.path().join("test.mlite");
    let adapter = IronBaseAdapter::new(db_path.to_string_lossy().to_string())
        .expect("Failed to create adapter");
    (Arc::new(adapter), temp_dir)
}

/// Dispatch a tool and expect success.
pub fn dispatch_ok(adapter: &Arc<IronBaseAdapter>, name: &str, params: Value) -> Value {
    dispatch_tool(name, params, adapter, None, None, None, None, &None, &None)
        .unwrap_or_else(|e| panic!("Tool '{}' should succeed: {:?}", name, e))
}

/// Dispatch a tool and expect an error.
pub fn dispatch_err(adapter: &Arc<IronBaseAdapter>, name: &str, params: Value) -> McpError {
    match dispatch_tool(name, params, adapter, None, None, None, None, &None, &None) {
        Ok(_) => panic!("Tool '{}' should fail", name),
        Err(e) => e,
    }
}
