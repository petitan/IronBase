//! HTTP Server module for IronBase MCP Server
//!
//! Provides HTTP server functionality that can be started with custom shutdown signals.

mod client;
mod config;
mod handler;
mod instructions;
mod logging;
mod response;
mod size;
mod state;
mod tls;

pub(crate) use client::{client_identity, is_client_initialized, mark_client_initialized};
pub use config::{load_config, Config};
pub(crate) use handler::handle_request;
pub(crate) use instructions::get_server_instructions;
pub(crate) use logging::SyncFileWriter;
pub(crate) use response::{create_error_response, create_success_response};
pub(crate) use size::format_size;
pub(crate) use state::HttpAppState;
pub(crate) use tls::load_rustls_config;

use crate::acl::AclManager;
use crate::transport::McpRequest;
use crate::{shutdown, ApiKeyCache, IronBaseAdapter, VERSION};
use std::sync::Arc;

/// Run HTTP server with default signal-based shutdown
pub async fn run_http_server() {
    run_http_server_internal(None, None).await;
}

/// Run HTTP server with an external shutdown receiver (used by Windows Service)
#[cfg(windows)]
pub async fn run_http_server_with_shutdown(shutdown_rx: std::sync::mpsc::Receiver<()>) {
    run_http_server_internal(Some(shutdown_rx), None).await;
}

/// Run HTTP server for Windows Service with shutdown + ready signal
#[cfg(windows)]
pub async fn run_http_server_for_service(
    shutdown_rx: std::sync::mpsc::Receiver<()>,
    ready_tx: std::sync::mpsc::SyncSender<()>,
) {
    run_http_server_internal(Some(shutdown_rx), Some(ready_tx)).await;
}

async fn run_http_server_internal(
    #[allow(unused_variables)] external_shutdown: Option<std::sync::mpsc::Receiver<()>>,
    #[allow(unused_variables)] ready_signal: Option<std::sync::mpsc::SyncSender<()>>,
) {
    use axum::{
        extract::{DefaultBodyLimit, State},
        http::StatusCode,
        response::{IntoResponse, Response},
        routing::{get, post},
        Json, Router,
    };
    use tokio::net::TcpListener;
    use tracing::{info, warn};
    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::util::SubscriberInitExt;

    // Load configuration FIRST (needed for logging config)
    let config = match load_config() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Failed to load configuration: {}", e);
            std::process::exit(1);
        }
    };

    // Initialize ironbase-core log level from config (before any core operations)
    // Priority: IRONBASE_LOG_LEVEL env var > config.toml [logging] core_level > default (warn)
    if let Some(ref level_str) = config.core_log_level {
        if let Some(level) = ironbase_core::LogLevel::from_str(level_str) {
            ironbase_core::set_log_level(level);
            eprintln!(
                "ℹ️ [CONFIG] ironbase-core log level set to {} from config.toml",
                level.as_str()
            );
        } else {
            eprintln!(
                "⚠️ [CONFIG] Invalid core_level '{}' in config.toml (valid: error, warn, info, debug, trace)",
                level_str
            );
        }
    }

    // Initialize tracing with dual output (stderr + file)
    // File logs go to ./logs/mcp-server.YYYY-MM-DD.log with daily rotation
    let env_filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));

    // Check if we're in debug mode (RUST_LOG contains "debug")
    let is_debug_mode = std::env::var("RUST_LOG")
        .map(|v| v.to_lowercase().contains("debug"))
        .unwrap_or(false);

    // Create logs directory - use IRONBASE_LOG_DIR or fallback to DB path's parent/logs
    // Use config.database_path (already loaded) instead of env var - works in Windows Service context
    let log_dir_path = std::env::var("IRONBASE_LOG_DIR").unwrap_or_else(|_| {
        if let Some(parent) = std::path::Path::new(&config.database_path).parent() {
            return parent.join("logs").to_string_lossy().to_string();
        }
        // Final fallback to current directory
        "./logs".to_string()
    });
    let log_dir = std::path::Path::new(&log_dir_path);
    if !log_dir.exists() {
        if let Err(e) = std::fs::create_dir_all(log_dir) {
            eprintln!("Warning: Failed to create log directory: {}", e);
        }
    }

    // Determine sync logging mode:
    // 1. IRONBASE_SYNC_LOG=1 env var (always respected as override)
    // 2. config.toml logging.sync=true when running in debug mode
    let env_sync = std::env::var("IRONBASE_SYNC_LOG")
        .map(|v| v == "1" || v.to_lowercase() == "true")
        .unwrap_or(false);
    let sync_logging = env_sync || (is_debug_mode && config.sync_logging);

    if sync_logging {
        // Sync logging: fsync after every write (slow but crash-safe)
        let today = chrono::Local::now().format("%Y-%m-%d");
        let log_path = format!("{}/mcp-server.{}.log", log_dir_path, today);

        let sync_writer = match SyncFileWriter::new(&log_path) {
            Ok(w) => w,
            Err(e) => {
                eprintln!("Failed to create sync log file: {}", e);
                std::process::exit(1);
            }
        };

        let _ = tracing_subscriber::registry()
            .with(env_filter)
            .with(
                tracing_subscriber::fmt::layer()
                    .with_writer(std::io::stderr)
                    .with_ansi(true),
            )
            .with(
                tracing_subscriber::fmt::layer()
                    .with_writer(sync_writer)
                    .with_ansi(false),
            )
            .try_init();

        eprintln!(
            "⚠️  SYNC LOGGING ENABLED - fsync after every log write (source: {})",
            if env_sync {
                "IRONBASE_SYNC_LOG env"
            } else {
                "config.toml + debug mode"
            }
        );
    } else {
        // Normal async logging (fast but may lose last logs on crash)
        // Use same filename format as sync mode: mcp-server.YYYY-MM-DD.log
        let today = chrono::Local::now().format("%Y-%m-%d");
        let log_path = format!("{}/mcp-server.{}.log", log_dir_path, today);
        let file = match std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)
        {
            Ok(f) => f,
            Err(e) => {
                eprintln!("Failed to create log file: {}", e);
                std::process::exit(1);
            }
        };
        let (non_blocking_file, _guard) = tracing_appender::non_blocking(file);

        let _ = tracing_subscriber::registry()
            .with(env_filter)
            .with(
                tracing_subscriber::fmt::layer()
                    .with_writer(std::io::stderr)
                    .with_ansi(true),
            )
            .with(
                tracing_subscriber::fmt::layer()
                    .with_writer(non_blocking_file)
                    .with_ansi(false),
            )
            .try_init();

        // Keep the guard alive for the duration of the server
        static LOG_GUARD: std::sync::OnceLock<tracing_appender::non_blocking::WorkerGuard> =
            std::sync::OnceLock::new();
        let _ = LOG_GUARD.set(_guard);
    }

    info!("Starting MCP IronBase Server v{} (HTTP mode)", VERSION);

    // Log initial memory stats
    crate::monitoring::log_memory_stats();

    let host = config.host.clone();
    let port = config.port;
    let max_body_size = config.max_body_size;
    let addr = format!("{}:{}", host, port);

    // FIX #3: Validate TLS config BEFORE creating adapter
    // This ensures we don't open the database only to exit() on TLS error
    let tls_config = if config.tls_enabled {
        let cert_file = match config.tls_cert_file.as_ref() {
            Some(path) => path.clone(),
            None => {
                tracing::error!("TLS enabled but tls.cert_file not set in config");
                std::process::exit(1);
            }
        };
        let key_file = match config.tls_key_file.as_ref() {
            Some(path) => path.clone(),
            None => {
                tracing::error!("TLS enabled but tls.key_file not set in config");
                std::process::exit(1);
            }
        };
        let rustls_config = match load_rustls_config(&cert_file, &key_file) {
            Ok(c) => c,
            Err(e) => {
                tracing::error!("Failed to load TLS config: {}", e);
                std::process::exit(1);
            }
        };
        let socket_addr: std::net::SocketAddr = match addr.parse() {
            Ok(a) => a,
            Err(e) => {
                tracing::error!("Invalid address '{}': {}", addr, e);
                std::process::exit(1);
            }
        };
        Some((rustls_config, socket_addr))
    } else {
        None
    };

    // Initialize IronBase adapter - only after config validation passes
    let adapter = match IronBaseAdapter::new(&config.database_path) {
        Ok(a) => Arc::new(a),
        Err(e) => {
            tracing::error!("Failed to create IronBase adapter: {}", e);
            std::process::exit(1);
        }
    };

    // Warm up collections (initialize index managers)
    // This moves the index rebuild cost from first query to startup
    adapter.warm_up();

    // Create API key cache
    let api_key_cache = ApiKeyCache::new(config.api_key_cache_ttl, config.require_api_key);

    // Pre-load API keys if required
    if config.require_api_key {
        api_key_cache.refresh(&adapter);
        info!(
            "API key authentication enabled (cache TTL: {}s)",
            config.api_key_cache_ttl
        );
    }

    // Create ACL manager
    let acl_manager = AclManager::new(adapter.clone());

    // Create dynamic limits manager (calculates limits based on available memory)
    let limits_manager = crate::LimitsManager::new();

    // Initialize embedding manager if FastText model is configured
    // Priority: IRONBASE_FASTTEXT_MODEL env var > config.toml [rag].fasttext_model
    let fasttext_path = std::env::var("IRONBASE_FASTTEXT_MODEL")
        .ok()
        .or_else(|| config.fasttext_model.clone());
    let embedding_manager: Option<Arc<crate::EmbeddingManager>> = if let Some(model_path) =
        fasttext_path
    {
        match crate::EmbeddingManager::with_fasttext(std::path::Path::new(&model_path)) {
            Ok(manager) if manager.has_providers() => {
                info!(
                    "Embedding manager initialized with FastText model: {}",
                    model_path
                );
                Some(Arc::new(manager))
            }
            Ok(_) => {
                warn!("FastText model configured but failed to load, embeddings disabled");
                None
            }
            Err(e) => {
                warn!("Failed to initialize embedding manager: {}", e);
                None
            }
        }
    } else {
        info!("No FastText model configured (env IRONBASE_FASTTEXT_MODEL or config [rag].fasttext_model), embeddings disabled");
        None
    };

    // Initialize job manager for async operations (embedding backfill, etc.)
    let job_manager = Arc::new(crate::JobManager::new());
    let job_manager_for_shutdown = job_manager.clone();
    let job_manager: Option<Arc<crate::JobManager>> = Some(job_manager);
    info!("Job manager initialized");

    // Lock working set on Windows to prevent memory paging under pressure.
    // Called after warm-up + FastText load so the floor captures all resident data.
    // Always enabled on Windows — no config needed. Graceful fallback if it fails.
    if cfg!(windows) {
        let ws_result = crate::memory_lock::lock_working_set();
        if ws_result.success {
            info!("{}", ws_result.message);
        } else {
            warn!("{}", ws_result.message);
        }
    }

    // Initialize listener configuration in database
    {
        use crate::listener::ListenerManager;
        let listener_manager = ListenerManager::new(adapter.clone());
        if let Err(e) = listener_manager.init_default(
            &host,
            port,
            config.tls_enabled,
            config.tls_cert_file.clone(),
            config.tls_key_file.clone(),
        ) {
            tracing::warn!("Failed to initialize default listener: {}", e);
        }
    }

    // Create server info
    let server_info = crate::ServerInfo {
        protocol: if config.tls_enabled {
            "https".to_string()
        } else {
            "http".to_string()
        },
        host: config.host.clone(),
        port: config.port,
        require_api_key: config.require_api_key,
    };

    // Create central service layer
    let service = Arc::new(crate::IronBaseService::new(
        adapter.clone(),
        acl_manager,
        api_key_cache,
        server_info,
        config.require_api_key,
        limits_manager.clone(),
        embedding_manager,
        job_manager,
    ));

    // Spawn periodic limits refresh task (every 5 minutes) with shutdown support
    let limits_for_refresh = limits_manager.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(300));
        // Create a separate shutdown signal listener for this task
        let shutdown = shutdown::shutdown_signal();
        tokio::pin!(shutdown);
        loop {
            tokio::select! {
                _ = interval.tick() => {
                    // IMPORTANT: refresh_if_needed() acquires parking_lot RwLock
                    // which blocks the thread. Must use spawn_blocking to avoid
                    // starving tokio worker threads.
                    let limits_clone = limits_for_refresh.clone();
                    if let Err(e) = tokio::task::spawn_blocking(move || {
                        limits_clone.refresh_if_needed();
                    }).await {
                        tracing::warn!("Limits refresh task panicked: {}", e);
                    }
                }
                _ = &mut shutdown => {
                    tracing::debug!("Limits refresh task received shutdown signal");
                    break;
                }
            }
        }
    });

    // Spawn periodic checkpoint task (every 60 seconds, MongoDB-style)
    // This ensures indexes are persisted to disk for crash recovery
    let adapter_for_checkpoint = adapter.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
        // Skip the first immediate tick
        interval.tick().await;
        // Create a separate shutdown signal listener for this task
        let shutdown = shutdown::shutdown_signal();
        tokio::pin!(shutdown);
        loop {
            tokio::select! {
                _ = interval.tick() => {
                    // IMPORTANT: checkpoint() acquires parking_lot write lock (db.write())
                    // which blocks the thread. Must use spawn_blocking to avoid
                    // starving tokio worker threads and freezing the entire runtime.
                    let adapter_clone = adapter_for_checkpoint.clone();
                    match tokio::task::spawn_blocking(move || adapter_clone.checkpoint_periodic()).await {
                        Ok(Ok(stats)) => {
                            let indexes = stats.get("indexes_flushed").and_then(|v| v.as_u64()).unwrap_or(0);
                            if indexes > 0 {
                                tracing::info!(
                                    indexes_flushed = indexes,
                                    "Periodic checkpoint completed"
                                );
                            }
                        }
                        Ok(Err(e)) => {
                            tracing::warn!("Periodic checkpoint failed: {}", e);
                        }
                        Err(e) => {
                            tracing::warn!("Checkpoint task panicked: {}", e);
                        }
                    }
                }
                _ = &mut shutdown => {
                    tracing::debug!("Checkpoint task received shutdown signal");
                    break;
                }
            }
        }
    });

    let app_state = Arc::new(HttpAppState {
        service,
        initialized_clients: std::sync::Mutex::new(std::collections::HashSet::new()),
        tool_timeout: std::time::Duration::from_secs(config.tool_timeout_secs),
        request_tracker: Arc::new(crate::cancellation::RequestTracker::new()),
    });

    // Sanitize request body for logging - mask API keys to prevent leakage
    fn sanitize_body_for_log(body: &str) -> String {
        let mut result = body.to_string();

        // Mask api_key values: "api_key":"value" or "api_key": "value"
        for key_pattern in &["\"api_key\":", "\"token\":", "\"authorization\":"] {
            if let Some(start) = result.to_lowercase().find(key_pattern) {
                // Find the value start (after the colon and optional whitespace/quote)
                let after_key = start + key_pattern.len();
                if let Some(rest) = result.get(after_key..) {
                    // Skip whitespace
                    let trimmed = rest.trim_start();
                    let skip_ws = rest.len() - trimmed.len();

                    if trimmed.starts_with('"') {
                        // Find closing quote
                        let value_start = after_key + skip_ws + 1; // after opening quote
                        if let Some(end_quote) = result[value_start..].find('"') {
                            let value_end = value_start + end_quote;
                            let value_len = value_end - value_start;
                            if value_len > 4 {
                                // Keep first 4 chars, mask rest
                                let masked =
                                    format!("{}****", &result[value_start..value_start + 4]);
                                result = format!(
                                    "{}\"{}\"{}",
                                    &result[..value_start - 1],
                                    masked,
                                    &result[value_end + 1..]
                                );
                            }
                        }
                    }
                }
            }
        }

        result
    }

    // HTTP request handler
    async fn http_handle_mcp_request(
        State(state): State<Arc<HttpAppState>>,
        axum::extract::ConnectInfo(remote_addr): axum::extract::ConnectInfo<std::net::SocketAddr>,
        headers: axum::http::HeaderMap,
        body: axum::body::Bytes,
    ) -> Response {
        // Generate unique request ID for tracing (first 8 chars of UUID)
        let trace_id = uuid::Uuid::new_v4().to_string()[..8].to_string();
        let request_start = std::time::Instant::now();

        // RAW request logging with trace ID (sanitized to prevent API key leakage)
        let body_str = String::from_utf8_lossy(&body);
        let sanitized_body = sanitize_body_for_log(&body_str);
        tracing::debug!(trace_id = %trace_id, remote = %remote_addr, ">>> MCP REQUEST: {}", sanitized_body);

        // Parse JSON
        let request: McpRequest = match serde_json::from_slice(&body) {
            Ok(req) => req,
            Err(e) => {
                let elapsed = request_start.elapsed();
                // Log detailed error internally, but return generic message to client
                tracing::error!(trace_id = %trace_id, elapsed_ms = elapsed.as_millis(), "<<< MCP PARSE ERROR: {}", e);
                return (StatusCode::BAD_REQUEST, "Invalid JSON request").into_response();
            }
        };

        // Handle notifications/cancelled immediately (before spawn_blocking)
        // This ensures cancellation is processed without waiting for tool execution
        if request.method == "notifications/cancelled" {
            let request_id = request
                .params
                .get("requestId")
                .and_then(|v| v.as_str())
                .or_else(|| request.params.get("request_id").and_then(|v| v.as_str()));
            let reason = request.params.get("reason").and_then(|v| v.as_str());

            if let Some(id) = request_id {
                state.request_tracker.cancel(id, reason);
            }
            // Notifications don't get responses
            return StatusCode::NO_CONTENT.into_response();
        }

        // Extract API key from Authorization header or JSON params
        let api_key = extract_api_key(&headers, &request.params);

        // Extract JSON-RPC request ID for cancellation tracking
        let json_rpc_id = request
            .id
            .as_ref()
            .map(|v| match v {
                serde_json::Value::String(s) => s.clone(),
                serde_json::Value::Number(n) => n.to_string(),
                _ => trace_id.clone(),
            })
            .unwrap_or_else(|| trace_id.clone());

        // Extract tool name for per-tool timeout (only for tools/call)
        let tool_name = if request.method == "tools/call" {
            request
                .params
                .get("name")
                .and_then(|n| n.as_str())
                .unwrap_or("unknown")
        } else {
            &request.method
        };

        // Calculate effective timeout: min(global, per-tool default)
        // Index creation operations return None (no timeout) to allow long-running builds
        let global_timeout_ms = state.tool_timeout.as_millis() as u64;
        let effective_timeout =
            crate::timeout::effective_timeout_option(global_timeout_ms, tool_name);

        // Register request for cancellation support
        let cancel_flag = state.request_tracker.start(&json_rpc_id);

        // Clone state for spawn_blocking closure
        let state_clone = state.clone();
        let request_id = request.id.clone();
        let request_method = request.method.clone();
        let tool_name_owned = tool_name.to_string();
        let trace_id_clone = trace_id.clone();
        let cancel_flag_clone = cancel_flag.clone();
        let json_rpc_id_for_cleanup = json_rpc_id.clone();
        let request_tracker_clone = state.request_tracker.clone();

        // Log incoming request with method and tool name
        let timeout_str = match effective_timeout {
            Some(t) => format!("{}s", t.as_secs()),
            None => "unlimited".to_string(),
        };
        tracing::info!(trace_id = %trace_id, method = %request_method, tool = %tool_name, timeout = %timeout_str, remote = %remote_addr, "Request started");

        // Run potentially blocking operations in spawn_blocking
        // Index creation operations run without timeout (effective_timeout = None)
        let deadline = effective_timeout.map(|t| std::time::Instant::now() + t);
        let effective_timeout_for_handler =
            effective_timeout.unwrap_or(std::time::Duration::from_secs(86400)); // Default 24h for no-timeout tools
        let mut handle = tokio::task::spawn_blocking(move || {
            // Set thread-local execution context for cooperative cancellation
            // For index operations, deadline is None - only client cancellation can stop them
            let ctx =
                crate::execution::ExecutionContext::new(deadline, Some(cancel_flag_clone.clone()));
            let _exec_guard = crate::execution::set_execution_context(ctx);

            handle_request(
                &request,
                &state_clone.service,
                &state_clone.initialized_clients,
                api_key.as_deref(),
                Some(remote_addr),
                effective_timeout_for_handler,
                Some(cancel_flag_clone),
            )
        });

        // For index operations (no timeout), wait indefinitely for completion
        // For other operations, use select! to enforce the timeout
        let result = if let Some(timeout_duration) = effective_timeout {
            tokio::select! {
                join_result = &mut handle => Ok(join_result),
                _ = tokio::time::sleep(timeout_duration) => {
                    // Timeout - signal cancellation and abort
                    cancel_flag.store(true, std::sync::atomic::Ordering::SeqCst);
                    handle.abort();
                    // Wait for task to fully stop to ensure resource cleanup
                    let _ = handle.await;
                    Err(())
                }
            }
        } else {
            // No timeout - wait indefinitely (index creation)
            // Client can still cancel via notifications/cancelled
            Ok(handle.await)
        };

        // Cleanup: unregister request from tracker
        request_tracker_clone.finish(&json_rpc_id_for_cleanup);

        let elapsed = request_start.elapsed();

        match result {
            Ok(Ok(Some(response))) => {
                // Success - normal response
                tracing::info!(
                    trace_id = %trace_id_clone,
                    method = %request_method,
                    tool = %tool_name_owned,
                    elapsed_ms = elapsed.as_millis(),
                    status = "success",
                    "Request completed"
                );
                if let Ok(json) = serde_json::to_string(&response) {
                    tracing::debug!(trace_id = %trace_id_clone, "<<< MCP RESPONSE: {}", json);
                }
                (StatusCode::OK, Json(response)).into_response()
            }
            Ok(Ok(None)) => {
                // Notification - no response body per JSON-RPC spec
                tracing::info!(
                    trace_id = %trace_id_clone,
                    method = %request_method,
                    tool = %tool_name_owned,
                    elapsed_ms = elapsed.as_millis(),
                    status = "notification",
                    "Request completed (no response)"
                );
                StatusCode::NO_CONTENT.into_response()
            }
            Ok(Err(join_error)) => {
                // spawn_blocking task panicked - log details internally, return generic to client
                tracing::error!(
                    trace_id = %trace_id_clone,
                    method = %request_method,
                    tool = %tool_name_owned,
                    elapsed_ms = elapsed.as_millis(),
                    status = "panic",
                    "<<< MCP ERROR: task panicked: {}", join_error
                );
                // Generic error message to prevent information leakage
                let error_response =
                    create_error_response(-32603, "Internal server error", request_id);
                (StatusCode::INTERNAL_SERVER_ERROR, Json(error_response)).into_response()
            }
            Err(_) => {
                // Timeout - operation took too long
                let timeout_secs = elapsed.as_secs();
                let error_msg = format!(
                    "Operation timed out after {} seconds. Tool: '{}'. \
                    Solutions: 1) Add 'limit' to your query, 2) Use an indexed field in your filter, \
                    3) Increase tool_timeout_secs in config.toml (or use faster tool).",
                    timeout_secs, tool_name_owned
                );
                tracing::error!(
                    trace_id = %trace_id_clone,
                    method = %request_method,
                    tool = %tool_name_owned,
                    elapsed_ms = elapsed.as_millis(),
                    status = "timeout",
                    "<<< MCP TIMEOUT: {}", error_msg
                );
                // Use proper MCP Timeout error code (-32008)
                let error_response = create_error_response(-32008, &error_msg, request_id);
                (StatusCode::GATEWAY_TIMEOUT, Json(error_response)).into_response()
            }
        }
    }

    /// Extract API key from Authorization header or JSON params
    /// SECURITY FIX #13: Case-insensitive header lookup with fallback
    fn extract_api_key(
        headers: &axum::http::HeaderMap,
        params: &serde_json::Value,
    ) -> Option<String> {
        // Try Authorization: Bearer header first (case-insensitive)
        // HTTP headers are case-insensitive per RFC 7230, hyper normalizes to lowercase
        // But we check both the standard constant and lowercase string for robustness
        let auth_header = headers
            .get(axum::http::header::AUTHORIZATION)
            .or_else(|| headers.get("authorization"));

        if let Some(auth_header) = auth_header {
            if let Ok(auth_str) = auth_header.to_str() {
                // Case-insensitive "Bearer " prefix check
                let auth_lower = auth_str.to_lowercase();
                if auth_lower.starts_with("bearer ") {
                    return Some(auth_str[7..].to_string());
                }
            }
        }

        // Fallback: check params.api_key
        if let Some(key) = params.get("api_key").and_then(|v| v.as_str()) {
            return Some(key.to_string());
        }

        // For tools/call, also check params.arguments.api_key
        if let Some(args) = params.get("arguments") {
            if let Some(key) = args.get("api_key").and_then(|v| v.as_str()) {
                return Some(key.to_string());
            }
        }

        None
    }

    async fn health_check() -> impl IntoResponse {
        let health = crate::monitoring::health_check();
        (StatusCode::OK, Json(health))
    }

    let app = Router::new()
        .route("/mcp", post(http_handle_mcp_request))
        .route("/health", get(health_check))
        .layer(DefaultBodyLimit::max(max_body_size))
        .with_state(app_state);

    // Create shutdown future based on source
    #[cfg(windows)]
    let shutdown_future: std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>> =
        if let Some(rx) = external_shutdown {
            Box::pin(async move {
                // Wait for signal from Windows SCM
                let _ = tokio::task::spawn_blocking(move || rx.recv()).await;
            })
        } else {
            Box::pin(shutdown::shutdown_signal())
        };

    #[cfg(not(windows))]
    let shutdown_future = shutdown::shutdown_signal();

    // Helper closure to close adapter and exit on fatal errors after adapter is created
    // FIX #3: Ensures database is properly closed before exit
    let fatal_exit = |adapter: &Arc<IronBaseAdapter>, msg: &str, e: &dyn std::fmt::Display| {
        tracing::error!("{}: {}", msg, e);
        if let Err(close_err) = adapter.close() {
            tracing::error!("Error closing database during fatal exit: {}", close_err);
        }
        std::process::exit(1);
    };

    // Run server - TLS or plain HTTP based on config
    // FIX #3: TLS config already validated before adapter creation
    if let Some((rustls_config, socket_addr)) = tls_config {
        info!(
            "Server listening on https://{} (TLS enabled, max body size: {})",
            addr,
            format_size(max_body_size)
        );

        // Signal ready to Windows Service SCM (server is accepting connections)
        #[allow(unused_variables)]
        if let Some(tx) = ready_signal {
            let _ = tx.send(());
        }

        // Use Handle for graceful shutdown with axum-server
        let handle = axum_server::Handle::new();
        let shutdown_handle = handle.clone();

        // Spawn a task to wait for shutdown signal
        tokio::spawn(async move {
            shutdown_future.await;
            shutdown_handle.graceful_shutdown(Some(std::time::Duration::from_secs(10)));
        });

        // Use into_make_service_with_connect_info to enable ConnectInfo extraction
        let app_service = app.into_make_service_with_connect_info::<std::net::SocketAddr>();
        if let Err(e) = axum_server::bind_rustls(socket_addr, rustls_config)
            .handle(handle)
            .serve(app_service)
            .await
        {
            tracing::error!("Server error: {}", e);
        }
    } else {
        // Plain HTTP mode with axum::serve
        let listener = match TcpListener::bind(&addr).await {
            Ok(l) => l,
            Err(e) => {
                fatal_exit(&adapter, &format!("Failed to bind to {}", addr), &e);
                unreachable!()
            }
        };

        info!(
            "Server listening on http://{} (max body size: {})",
            addr,
            format_size(max_body_size)
        );

        // Signal ready to Windows Service SCM (server is accepting connections)
        #[allow(unused_variables)]
        if let Some(tx) = ready_signal {
            let _ = tx.send(());
        }

        // Use into_make_service_with_connect_info to enable ConnectInfo extraction
        let app_service = app.into_make_service_with_connect_info::<std::net::SocketAddr>();

        // Drain timeout: if in-flight requests don't complete within 10s
        // after shutdown signal, force-close connections and proceed to close().
        // Without this, a stuck spawn_blocking task blocks axum forever.
        let (drain_tx, drain_rx) = tokio::sync::oneshot::channel::<()>();
        let graceful_shutdown = async move {
            shutdown_future.await;
            let _ = drain_tx.send(());
        };
        let serve_future =
            axum::serve(listener, app_service).with_graceful_shutdown(graceful_shutdown);

        tokio::select! {
            result = serve_future => {
                if let Err(e) = result {
                    tracing::error!("Server error: {}", e);
                }
            }
            _ = async {
                let _ = drain_rx.await;
                info!("Shutdown signal received, draining connections (10s deadline)...");
                tokio::time::sleep(std::time::Duration::from_secs(10)).await;
            } => {
                warn!("Connection drain timed out after 10s, forcing server close");
            }
        }
    }

    // Graceful shutdown: stop background jobs first
    info!("Shutting down gracefully...");

    // 1. Stop all background jobs and wait for threads to complete
    let completed = job_manager_for_shutdown.shutdown();
    if completed > 0 {
        info!("Stopped {} background job threads", completed);
    }

    // 2. Close database (flush indexes + mark clean shutdown)
    if let Err(e) = adapter.close() {
        tracing::error!("Error closing database: {}", e);
    } else {
        info!("Database closed cleanly - fast restart enabled");
    }
    info!("Server stopped");
}
