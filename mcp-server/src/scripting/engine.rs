//! Rhai script execution engine.
//!
//! Provides the core execution engine with:
//! - Resource limits (operations, memory, time)
//! - Log collection with size limits
//! - Result size validation
//! - Graceful cancellation support

use crate::adapter::IronBaseAdapter;
use crate::embedding::EmbeddingManager;
use crate::error::{McpError, Result};
use parking_lot::Mutex;
use rhai::{Dynamic, Engine, Scope};
use serde_json::Value;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::conversion::{dynamic_to_json, json_to_dynamic};
use super::db_functions::register_db_functions;
use super::error::{format_rhai_error, is_operation_limit_error};
use super::limits::{
    estimate_json_size, LogCollector, ScriptLimits, DEFAULT_MAX_OPERATIONS, MAX_OPERATIONS,
    MAX_SCRIPT_COLLECTION_SIZE, MAX_SCRIPT_STRING_SIZE, MEMORY_CHECK_INTERVAL,
};
use super::types::ScriptResult;
use super::utility_functions::register_utility_functions;

/// The operation limit for a run: the caller's value (default if 0), capped
/// at `MAX_OPERATIONS` (MCP audit 2026-10-14 M4).
fn effective_max_operations(limits: &ScriptLimits) -> u64 {
    if limits.max_operations > 0 {
        limits.max_operations.min(MAX_OPERATIONS)
    } else {
        DEFAULT_MAX_OPERATIONS
    }
}

/// Apply the resource limits to a Rhai engine; returns the operation limit.
fn configure_limits(engine: &mut Engine, limits: &ScriptLimits) -> u64 {
    // Operation limit (default if 0, capped to prevent DoS)
    let max_ops = effective_max_operations(limits);
    engine.set_max_operations(max_ops);

    // Value size limits: a string or collection doubled a few times
    // aborted the process on allocation failure (M5)
    engine.set_max_string_size(limits.max_result_size.min(MAX_SCRIPT_STRING_SIZE));
    engine.set_max_array_size(MAX_SCRIPT_COLLECTION_SIZE);
    engine.set_max_map_size(MAX_SCRIPT_COLLECTION_SIZE);
    max_ops
}

/// Reads the bytes the process has allocated; `None` if not measurable.
type MemoryProbe = fn() -> Option<u64>;

/// Process allocation from the allocator statistics (jemalloc on Unix,
/// `GetProcessMemoryInfo` on Windows).
/// A failed statistics read reports 0, which a running process never has.
fn process_allocated_bytes() -> Option<u64> {
    let stats = crate::monitoring::get_memory_stats();
    (stats.available && stats.allocated_bytes > 0).then_some(stats.allocated_bytes)
}

/// Memory budget of one script run: half of the memory available at its
/// start, from the cgroup limit when the server runs in a container. Per-value
/// limits do not bound a script that keeps many values alive (M5 follow-up).
/// `None` if memory is not measurable here (total reported as 0); a genuine 0
/// available gives a 0 budget.
fn script_memory_budget() -> Option<u64> {
    let mut sys = sysinfo::System::new();
    sys.refresh_memory();
    if sys.total_memory() == 0 {
        return None;
    }
    let mut available = sys.available_memory();
    if let Some(cgroup) = sys.cgroup_limits() {
        available = available.min(cgroup.free_memory);
    }
    Some(available / 2)
}

fn memory_budget_error(budget: Option<u64>) -> McpError {
    McpError::script_error(format!(
        "Script stopped: process memory grew past its budget ({} MB, half of \
        the memory available at start). Keep fewer values alive or use pagination.",
        budget.unwrap_or(0) / (1024 * 1024)
    ))
}

/// Rhai script execution engine with IronBase bindings.
///
/// Executes Rhai scripts with database access and resource limits.
/// All resource limits are enforced to prevent OOM and DoS attacks.
///
/// # Example
///
/// ```rust,ignore
/// let engine = RhaiEngine::new(adapter, None);
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
///
/// // With embedding manager for RAG functions
/// let engine = RhaiEngine::new(adapter, Some(embedding_manager));
/// let result = engine.run("db_hybrid_search(\"kb\", \"query\")", None)?;
/// ```
pub struct RhaiEngine {
    adapter: Arc<IronBaseAdapter>,
    embedding_manager: Option<Arc<EmbeddingManager>>,
    memory_probe: MemoryProbe,
}

impl RhaiEngine {
    /// Create a new RhaiEngine.
    ///
    /// # Arguments
    ///
    /// * `adapter` - Database adapter for db_* functions
    /// * `embedding_manager` - Optional embedding manager for db_rag_* functions
    pub fn new(
        adapter: Arc<IronBaseAdapter>,
        embedding_manager: Option<Arc<EmbeddingManager>>,
    ) -> Self {
        Self {
            adapter,
            embedding_manager,
            memory_probe: process_allocated_bytes,
        }
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
        let max_ops = configure_limits(&mut engine, limits);

        // Stop on cancellation (own flag or the MCP request's), and at the
        // timeout: `timeout_ms` was never enforced on this path (M4)
        let started = Instant::now();
        let time_limit = (limits.timeout_ms > 0).then(|| Duration::from_millis(limits.timeout_ms));
        let request_cancel = crate::execution::current_cancel_flag();
        let timed_out = Arc::new(AtomicBool::new(false));
        let timed_out_flag = timed_out.clone();
        let cancelled_check = cancelled.clone();
        let request_cancel_check = request_cancel.clone();

        // Stop when the process memory grew past the script's budget (M5
        // follow-up), checked every MEMORY_CHECK_INTERVAL
        let probe = self.memory_probe;
        let memory_start = probe();
        let memory_budget = memory_start.and(script_memory_budget());
        let memory_exceeded = Arc::new(AtomicBool::new(false));
        let memory_exceeded_flag = memory_exceeded.clone();
        let last_memory_check_ms = Arc::new(AtomicU64::new(0));
        engine.on_progress(move |ops| {
            if ops % 1024 == 0 && time_limit.is_some_and(|t| started.elapsed() >= t) {
                timed_out_flag.store(true, Ordering::Relaxed);
            }
            if let (Some(start), Some(budget), 0) = (memory_start, memory_budget, ops % 16) {
                let now_ms = started.elapsed().as_millis() as u64;
                let interval_ms = MEMORY_CHECK_INTERVAL.as_millis() as u64;
                if now_ms >= last_memory_check_ms.load(Ordering::Relaxed) + interval_ms {
                    last_memory_check_ms.store(now_ms, Ordering::Relaxed);
                    if probe().is_some_and(|now| now.saturating_sub(start) > budget) {
                        memory_exceeded_flag.store(true, Ordering::Relaxed);
                    }
                }
            }
            if timed_out_flag.load(Ordering::Relaxed)
                || memory_exceeded_flag.load(Ordering::Relaxed)
                || cancelled_check.load(Ordering::Relaxed)
                || request_cancel_check
                    .as_ref()
                    .is_some_and(|f| f.load(Ordering::Relaxed))
            {
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

        // Register database functions with limits (including RAG functions if embedding_manager is available)
        register_db_functions(
            &mut engine,
            self.adapter.clone(),
            self.embedding_manager.clone(),
            limits,
        );

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

        // A final check: growth after the last periodic probe must not slip
        // through on a script that finishes before the next one
        if let (Some(start), Some(budget)) = (memory_start, memory_budget) {
            if probe().is_some_and(|now| now.saturating_sub(start) > budget) {
                memory_exceeded.store(true, Ordering::Relaxed);
            }
        }

        match result {
            Ok(value) => {
                if memory_exceeded.load(Ordering::Relaxed) {
                    return Err(memory_budget_error(memory_budget));
                }
                // Check if cancelled during execution
                if cancelled.load(Ordering::Relaxed) {
                    return Err(McpError::script_error("Script was cancelled".to_string()));
                }

                let json_result = dynamic_to_json(&value);

                // Check result size limit
                let result_size = estimate_json_size(&json_result);
                if limits.result_exceeds_limit(result_size) {
                    return Err(McpError::script_error(format!(
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
                if cancelled.load(Ordering::Relaxed)
                    || request_cancel
                        .as_ref()
                        .is_some_and(|f| f.load(Ordering::Relaxed))
                {
                    return Err(McpError::script_error("Script was cancelled".to_string()));
                }
                // Set only by the progress callback: an error raised for
                // another reason after the deadline is not relabeled
                if memory_exceeded.load(Ordering::Relaxed) {
                    return Err(memory_budget_error(memory_budget));
                }
                if timed_out.load(Ordering::Relaxed) {
                    return Err(McpError::script_error(format!(
                        "Script execution timed out after {} ms",
                        limits.timeout_ms
                    )));
                }

                let err_str = format_rhai_error(&e);

                // Check for operation limit
                if is_operation_limit_error(&e) {
                    Err(McpError::script_error(format!(
                        "Script exceeded maximum operations limit ({})",
                        max_ops
                    )))
                } else {
                    Err(McpError::script_error(err_str))
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
        let embedding_manager = self.embedding_manager.clone();
        let limits = limits.clone();
        let params = params.clone();

        // The request's execution context (cancel flag) is thread-local:
        // carry it to the worker thread
        let exec_ctx = crate::execution::current_execution_context();

        // Spawn blocking task for Rhai execution
        let handle = tokio::task::spawn_blocking(move || {
            let _exec_guard = exec_ctx.map(crate::execution::set_execution_context);
            let engine = RhaiEngine::new(adapter, embedding_manager);
            engine.run_with_cancellation(&code, params, &limits, cancelled_clone)
        });

        // Apply timeout
        match tokio::time::timeout(timeout_duration, handle).await {
            Ok(Ok(result)) => result,
            Ok(Err(join_err)) => Err(McpError::script_error(format!(
                "Script task panicked: {}",
                join_err
            ))),
            Err(_elapsed) => {
                // Timeout - signal cancellation
                cancelled.store(true, Ordering::SeqCst);
                Err(McpError::script_error(format!(
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
        let engine = RhaiEngine::new(adapter, None);

        let result = engine.run("1 + 2", None).unwrap();
        assert_eq!(result.result, json!(3));
    }

    #[test]
    fn test_with_params() {
        let (adapter, _temp) = create_test_adapter();
        let engine = RhaiEngine::new(adapter, None);

        let result = engine
            .run("params.x + params.y", Some(json!({"x": 10, "y": 5})))
            .unwrap();
        assert_eq!(result.result, json!(15));
    }

    #[test]
    fn test_print_captures_logs() {
        let (adapter, _temp) = create_test_adapter();
        let engine = RhaiEngine::new(adapter, None);

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
        let engine = RhaiEngine::new(adapter, None);

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
        let engine = RhaiEngine::new(adapter, None);

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
        let engine = RhaiEngine::new(adapter, None);

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
        let engine = RhaiEngine::new(adapter, None);

        let cancelled = Arc::new(AtomicBool::new(true)); // Pre-cancelled

        let result =
            engine.run_with_cancellation("1 + 1", None, &ScriptLimits::default(), cancelled);

        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("cancelled"));
    }

    /// M4: an infinite loop with a huge `max_operations` ends at the timeout
    /// (it used to run until the process was killed).
    #[test]
    fn test_timeout_ends_infinite_loop() {
        let (adapter, _temp) = create_test_adapter();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let engine = RhaiEngine::new(adapter, None);
            let limits = ScriptLimits {
                max_operations: u64::MAX,
                timeout_ms: 200,
                ..ScriptLimits::default()
            };
            let _ = tx.send(engine.run_with_limits("loop {}", None, &limits));
        });
        // Well below the 100M-operation cap: the callback stopped it
        let result = rx
            .recv_timeout(Duration::from_secs(5))
            .expect("script did not stop at its timeout");
        let err = result.unwrap_err().to_string();
        assert!(err.contains("timed out"), "{err}");
    }

    /// M4: `max_operations` from the caller is capped on the engine.
    #[test]
    fn test_max_operations_is_capped() {
        let limits = ScriptLimits {
            max_operations: u64::MAX,
            ..ScriptLimits::default()
        };
        let mut engine = Engine::new();
        assert_eq!(configure_limits(&mut engine, &limits), MAX_OPERATIONS);
        assert_eq!(engine.max_operations(), MAX_OPERATIONS);
        assert_eq!(
            engine.max_string_size(),
            limits.max_result_size.min(MAX_SCRIPT_STRING_SIZE)
        );
        assert_eq!(engine.max_array_size(), MAX_SCRIPT_COLLECTION_SIZE);
        assert_eq!(engine.max_map_size(), MAX_SCRIPT_COLLECTION_SIZE);
    }

    /// M5 follow-up: a script keeping many allowed values alive is stopped
    /// when the process memory grows past its budget. The probe is faked
    /// (tests run without jemalloc): it reports 1 GB more on every read.
    #[test]
    fn test_memory_growth_stops_script() {
        static READS: AtomicU64 = AtomicU64::new(0);
        fn growing_probe() -> Option<u64> {
            Some(READS.fetch_add(1, Ordering::Relaxed) * (1 << 30))
        }
        let (adapter, _temp) = create_test_adapter();
        let mut engine = RhaiEngine::new(adapter, None);
        engine.memory_probe = growing_probe;
        let limits = ScriptLimits {
            max_operations: MAX_OPERATIONS,
            timeout_ms: 0,
            ..ScriptLimits::default()
        };
        let err = engine
            .run_with_limits("let x = 0; loop { x += 1; }", None, &limits)
            .unwrap_err()
            .to_string();
        assert!(err.contains("memory"), "{err}");
    }

    /// M5 follow-up: growth past the budget after the last periodic probe is
    /// caught by the final check of a script that finishes quickly.
    #[test]
    fn test_final_memory_check_rejects_result() {
        static READS: AtomicU64 = AtomicU64::new(0);
        fn jump_probe() -> Option<u64> {
            // First read (start): 1; later reads: far past any budget
            Some(if READS.fetch_add(1, Ordering::Relaxed) == 0 {
                1
            } else {
                u64::MAX / 2
            })
        }
        let (adapter, _temp) = create_test_adapter();
        let mut engine = RhaiEngine::new(adapter, None);
        engine.memory_probe = jump_probe;
        let err = engine.run("1 + 2", None).unwrap_err().to_string();
        assert!(err.contains("memory"), "{err}");
    }

    /// M5 follow-up: one string is capped at MAX_SCRIPT_STRING_SIZE even
    /// with a larger result limit.
    #[test]
    fn test_string_size_capped_below_result_limit() {
        let limits = ScriptLimits {
            max_result_size: 100 * 1024 * 1024,
            ..ScriptLimits::default()
        };
        let mut engine = Engine::new();
        configure_limits(&mut engine, &limits);
        assert_eq!(engine.max_string_size(), MAX_SCRIPT_STRING_SIZE);
    }

    /// M4: a cancelled MCP request stops its script.
    #[test]
    fn test_request_cancel_stops_script() {
        let (adapter, _temp) = create_test_adapter();
        let flag = Arc::new(AtomicBool::new(true));
        let _guard = crate::execution::set_execution_context(
            crate::execution::ExecutionContext::new(None, Some(flag)),
        );
        let engine = RhaiEngine::new(adapter, None);
        let err = engine.run("loop {}", None).unwrap_err().to_string();
        assert!(err.contains("cancelled"), "{err}");
    }

    /// M5: a string doubled past the size limit fails the script instead of
    /// allocating without bound (40 doublings would need ~8 TB).
    #[test]
    fn test_string_size_is_limited() {
        let (adapter, _temp) = create_test_adapter();
        let engine = RhaiEngine::new(adapter, None);
        let result = engine.run(
            r#"let s = "xxxxxxxx"; for i in 0..25 { s += s; } s.len()"#,
            None,
        );
        assert!(result.is_err(), "256 MB string built: {result:?}");
    }
}
