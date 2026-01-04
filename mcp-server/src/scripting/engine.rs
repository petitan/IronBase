//! Rhai script execution engine.
//!
//! Provides the core execution engine with:
//! - Resource limits (operations, memory, time)
//! - Log collection with size limits
//! - Result size validation
//! - Graceful cancellation support

use crate::adapter::IronBaseAdapter;
use crate::error::{McpError, Result};
use parking_lot::Mutex;
use rhai::{Dynamic, Engine, Scope};
use serde_json::Value;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::conversion::{dynamic_to_json, json_to_dynamic};
use super::db_functions::register_db_functions;
use super::error::{format_rhai_error, is_operation_limit_error};
use super::limits::{estimate_json_size, LogCollector, ScriptLimits, DEFAULT_MAX_OPERATIONS};
use super::types::ScriptResult;
use super::utility_functions::register_utility_functions;

/// Rhai script execution engine with IronBase bindings.
///
/// Executes Rhai scripts with database access and resource limits.
/// All resource limits are enforced to prevent OOM and DoS attacks.
///
/// # Example
///
/// ```rust,ignore
/// let engine = RhaiEngine::new(adapter);
///
/// // Simple execution
/// let result = engine.run("1 + 2", None)?;
/// assert_eq!(result.result, json!(3));
///
/// // With parameters
/// let result = engine.run("params.x * 2", Some(json!({"x": 21})))?;
/// assert_eq!(result.result, json!(42));
///
/// // With custom limits
/// let limits = ScriptLimits::with_timeout(5000); // 5 seconds
/// let result = engine.run_with_limits("long_computation()", None, &limits)?;
/// ```
pub struct RhaiEngine {
    adapter: Arc<IronBaseAdapter>,
}

impl RhaiEngine {
    /// Create a new RhaiEngine.
    ///
    /// # Arguments
    ///
    /// * `adapter` - Database adapter for db_* functions
    pub fn new(adapter: Arc<IronBaseAdapter>) -> Self {
        Self { adapter }
    }

    /// Run a script with optional parameters (uses default limits).
    ///
    /// # Arguments
    ///
    /// * `code` - Rhai script code
    /// * `params` - Optional parameters accessible as `params` in script
    ///
    /// # Returns
    ///
    /// Execution result with logs and timing
    pub fn run(&self, code: &str, params: Option<Value>) -> Result<ScriptResult> {
        self.run_with_limits(code, params, &ScriptLimits::default())
    }

    /// Run a script with custom resource limits.
    ///
    /// # Arguments
    ///
    /// * `code` - Rhai script code
    /// * `params` - Optional parameters
    /// * `limits` - Resource limits for execution
    ///
    /// # Returns
    ///
    /// Execution result with logs and timing
    ///
    /// # Errors
    ///
    /// - `McpError::ScriptError` - Script syntax/runtime error
    /// - `McpError::ScriptError` - Operation limit exceeded
    /// - `McpError::ScriptError` - Result size limit exceeded
    pub fn run_with_limits(
        &self,
        code: &str,
        params: Option<Value>,
        limits: &ScriptLimits,
    ) -> Result<ScriptResult> {
        self.run_internal(code, params, limits, Arc::new(AtomicBool::new(false)))
    }

    /// Run a script with dependencies.
    ///
    /// Dependencies are concatenated before the main script.
    ///
    /// # Arguments
    ///
    /// * `code` - Main script code
    /// * `dependency_codes` - Dependency scripts (in topological order)
    /// * `params` - Optional parameters
    /// * `limits` - Resource limits
    pub fn run_with_dependencies(
        &self,
        code: &str,
        dependency_codes: Vec<String>,
        params: Option<Value>,
        limits: &ScriptLimits,
    ) -> Result<ScriptResult> {
        // Concatenate dependency code before main script
        let full_code = if dependency_codes.is_empty() {
            code.to_string()
        } else {
            let deps = dependency_codes.join("\n\n");
            format!("{}\n\n// === Main Script ===\n{}", deps, code)
        };

        self.run_with_limits(&full_code, params, limits)
    }

    /// Run a script with cancellation support.
    ///
    /// Used for timeout and graceful shutdown.
    ///
    /// # Arguments
    ///
    /// * `code` - Script code
    /// * `params` - Optional parameters
    /// * `limits` - Resource limits
    /// * `cancelled` - Atomic flag for cancellation
    pub fn run_with_cancellation(
        &self,
        code: &str,
        params: Option<Value>,
        limits: &ScriptLimits,
        cancelled: Arc<AtomicBool>,
    ) -> Result<ScriptResult> {
        self.run_internal(code, params, limits, cancelled)
    }

    /// Internal execution implementation.
    fn run_internal(
        &self,
        code: &str,
        params: Option<Value>,
        limits: &ScriptLimits,
        cancelled: Arc<AtomicBool>,
    ) -> Result<ScriptResult> {
        let mut engine = Engine::new();

        // Set operation limit (use default if 0 to prevent DoS)
        let max_ops = if limits.max_operations > 0 {
            limits.max_operations
        } else {
            DEFAULT_MAX_OPERATIONS
        };
        engine.set_max_operations(max_ops);

        // Setup cancellation check via progress callback
        let cancelled_check = cancelled.clone();
        engine.on_progress(move |_ops| {
            if cancelled_check.load(Ordering::Relaxed) {
                // Signal termination
                Some(rhai::Dynamic::UNIT)
            } else {
                None // Continue execution
            }
        });

        // Create log collector with limits
        let log_collector = Arc::new(Mutex::new(LogCollector::new(limits)));
        let log_collector_clone = log_collector.clone();

        // Register print function with limit enforcement
        engine.on_print(move |s| {
            log_collector_clone.lock().push(s);
        });

        // Register database functions with limits
        register_db_functions(&mut engine, self.adapter.clone(), limits);

        // Register utility functions
        register_utility_functions(&mut engine);

        // Create scope with params
        let mut scope = Scope::new();
        if let Some(params) = params {
            let params_dynamic = json_to_dynamic(&params);
            scope.push("params", params_dynamic);
        }

        // Execute with timing
        let start = Instant::now();
        let result = engine.eval_with_scope::<Dynamic>(&mut scope, code);
        let execution_time_ms = start.elapsed().as_millis() as u64;

        // Finalize logs
        let (logs, log_warning) = log_collector.lock().finalize();
        let mut final_logs = logs;
        if let Some(warning) = log_warning {
            final_logs.push(warning);
        }

        match result {
            Ok(value) => {
                // Check if cancelled during execution
                if cancelled.load(Ordering::Relaxed) {
                    return Err(McpError::ScriptError("Script was cancelled".to_string()));
                }

                let json_result = dynamic_to_json(&value);

                // Check result size limit
                let result_size = estimate_json_size(&json_result);
                if limits.result_exceeds_limit(result_size) {
                    return Err(McpError::ScriptError(format!(
                        "Script result too large ({} bytes > {} bytes max). \
                        Return smaller data or use pagination.",
                        result_size, limits.max_result_size
                    )));
                }

                Ok(ScriptResult {
                    result: json_result,
                    logs: final_logs,
                    execution_time_ms,
                })
            }
            Err(e) => {
                // Check if it was a cancellation
                if cancelled.load(Ordering::Relaxed) {
                    return Err(McpError::ScriptError("Script was cancelled".to_string()));
                }

                let err_str = format_rhai_error(&e);

                // Check for operation limit
                if is_operation_limit_error(&e) {
                    Err(McpError::ScriptError(format!(
                        "Script exceeded maximum operations limit ({})",
                        max_ops
                    )))
                } else {
                    Err(McpError::ScriptError(err_str))
                }
            }
        }
    }

    /// Run a script with timeout (async).
    ///
    /// Uses tokio for async timeout handling.
    ///
    /// # Arguments
    ///
    /// * `code` - Script code
    /// * `params` - Optional parameters
    /// * `limits` - Resource limits (timeout_ms is used)
    ///
    /// # Returns
    ///
    /// Execution result or timeout error
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let limits = ScriptLimits::with_timeout(5000);
    /// let result = engine.run_with_timeout("long_task()", None, &limits).await?;
    /// ```
    pub async fn run_with_timeout(
        &self,
        code: &str,
        params: Option<Value>,
        limits: &ScriptLimits,
    ) -> Result<ScriptResult> {
        let timeout_ms = limits.timeout_ms;
        let timeout_duration = Duration::from_millis(timeout_ms);
        let cancelled = Arc::new(AtomicBool::new(false));
        let cancelled_clone = cancelled.clone();

        let code = code.to_string();
        let adapter = self.adapter.clone();
        let limits = limits.clone();
        let params = params.clone();

        // Spawn blocking task for Rhai execution
        let handle = tokio::task::spawn_blocking(move || {
            let engine = RhaiEngine::new(adapter);
            engine.run_with_cancellation(&code, params, &limits, cancelled_clone)
        });

        // Apply timeout
        match tokio::time::timeout(timeout_duration, handle).await {
            Ok(Ok(result)) => result,
            Ok(Err(join_err)) => Err(McpError::ScriptError(format!(
                "Script task panicked: {}",
                join_err
            ))),
            Err(_elapsed) => {
                // Timeout - signal cancellation
                cancelled.store(true, Ordering::SeqCst);
                Err(McpError::ScriptError(format!(
                    "Script execution timed out after {} ms",
                    timeout_ms
                )))
            }
        }
    }
}

// ============================================================
// Legacy Compatibility
// ============================================================

impl RhaiEngine {
    /// Run with ScriptOptions (legacy API).
    ///
    /// Converts ScriptOptions to ScriptLimits internally.
    pub fn run_with_options(
        &self,
        code: &str,
        params: Option<Value>,
        options: super::types::ScriptOptions,
    ) -> Result<ScriptResult> {
        let limits: ScriptLimits = options.into();
        self.run_with_limits(code, params, &limits)
    }

    /// Run with dependencies and options (legacy API).
    pub fn run_with_dependencies_opts(
        &self,
        code: &str,
        dependency_codes: Vec<String>,
        params: Option<Value>,
        options: Option<super::types::ScriptOptions>,
    ) -> Result<ScriptResult> {
        let limits = options.map(Into::into).unwrap_or_default();
        self.run_with_dependencies(code, dependency_codes, params, &limits)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tempfile::TempDir;

    fn create_test_adapter() -> (Arc<IronBaseAdapter>, TempDir) {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.mlite");
        let adapter = Arc::new(IronBaseAdapter::new(&db_path).unwrap());
        (adapter, temp_dir)
    }

    #[test]
    fn test_simple_arithmetic() {
        let (adapter, _temp) = create_test_adapter();
        let engine = RhaiEngine::new(adapter);

        let result = engine.run("1 + 2", None).unwrap();
        assert_eq!(result.result, json!(3));
    }

    #[test]
    fn test_with_params() {
        let (adapter, _temp) = create_test_adapter();
        let engine = RhaiEngine::new(adapter);

        let result = engine
            .run("params.x + params.y", Some(json!({"x": 10, "y": 5})))
            .unwrap();
        assert_eq!(result.result, json!(15));
    }

    #[test]
    fn test_print_captures_logs() {
        let (adapter, _temp) = create_test_adapter();
        let engine = RhaiEngine::new(adapter);

        let result = engine
            .run(
                r#"
                print("Hello");
                print("World");
                42
            "#,
                None,
            )
            .unwrap();

        assert_eq!(result.result, json!(42));
        assert_eq!(result.logs.len(), 2);
        assert_eq!(result.logs[0], "Hello");
        assert_eq!(result.logs[1], "World");
    }

    #[test]
    fn test_db_operations() {
        let (adapter, _temp) = create_test_adapter();
        let engine = RhaiEngine::new(adapter);

        let result = engine
            .run(
                r#"
                let doc = #{ name: "Alice", age: 30 };
                let id = db_insert_one("users", doc);
                let found = db_find("users", #{});
                found.documents.len()
            "#,
                None,
            )
            .unwrap();

        assert_eq!(result.result, json!(1));
    }

    #[test]
    fn test_result_size_limit() {
        let (adapter, _temp) = create_test_adapter();
        let engine = RhaiEngine::new(adapter);

        // Create limits with very small result size
        let limits = ScriptLimits {
            max_result_size: 10, // 10 bytes max
            ..Default::default()
        };

        // This should exceed the limit
        let result = engine.run_with_limits(r#"[1, 2, 3, 4, 5, 6, 7, 8, 9, 10]"#, None, &limits);

        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("too large"));
    }

    #[test]
    fn test_log_truncation() {
        let (adapter, _temp) = create_test_adapter();
        let engine = RhaiEngine::new(adapter);

        let limits = ScriptLimits {
            max_log_entry_size: 10, // 10 bytes per entry
            ..Default::default()
        };

        let result = engine
            .run_with_limits(
                r#"print("This is a very long message that exceeds the limit"); 1"#,
                None,
                &limits,
            )
            .unwrap();

        assert!(result.logs[0].contains("TRUNCATED"));
    }

    #[test]
    fn test_cancellation() {
        let (adapter, _temp) = create_test_adapter();
        let engine = RhaiEngine::new(adapter);

        let cancelled = Arc::new(AtomicBool::new(true)); // Pre-cancelled

        let result =
            engine.run_with_cancellation("1 + 1", None, &ScriptLimits::default(), cancelled);

        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("cancelled"));
    }
}
