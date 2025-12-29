//! HTTP Server module for IronBase MCP Server
//!
//! Provides HTTP server functionality that can be started with custom shutdown signals.

use crate::acl::{AclManager, CallerContext};
use crate::{shutdown, ApiKeyCache, IronBaseAdapter, VERSION};
use std::path::PathBuf;
use std::sync::Arc;

/// Default max body size: 1 GB
const DEFAULT_MAX_BODY_SIZE: usize = 1024 * 1024 * 1024;

///// Default tool timeout: 60 seconds
/// SAFETY: Reduced from 300s to prevent WSL freezes on runaway queries
/// If a query takes longer than 60s, it should be optimized with indexes/limits
const DEFAULT_TOOL_TIMEOUT_SECS: u64 = 60;

/// Configuration for HTTP server
#[derive(Debug, Clone)]
pub struct Config {
    pub host: String,
    pub port: u16,
    pub database_path: PathBuf,
    pub max_body_size: usize,
    /// If true, API key is required for all tool calls
    pub require_api_key: bool,
    /// Cache TTL for API keys in seconds
    pub api_key_cache_ttl: u64,
    /// If true, use HTTPS instead of HTTP
    pub tls_enabled: bool,
    /// Path to TLS certificate file
    pub tls_cert_file: Option<String>,
    /// Path to TLS key file
    pub tls_key_file: Option<String>,
    /// Timeout for long-running tool operations in seconds (default: 300)
    pub tool_timeout_secs: u64,
}

/// Parse human-readable size strings like "1GB", "500MB", "10KB"
///
/// Supported suffixes (case-insensitive):
/// - B (bytes)
/// - KB (kilobytes, 1024 bytes)
/// - MB (megabytes, 1024^2 bytes)
/// - GB (gigabytes, 1024^3 bytes)
///
/// Examples: "1GB", "500MB", "10KB", "1024B", "1 GB" (spaces allowed)
fn parse_size(s: &str) -> Result<usize, String> {
    let s = s.trim().to_uppercase();

    // Find where the number ends and the suffix begins
    let (num_part, suffix) = if let Some(pos) = s.find(|c: char| c.is_alphabetic()) {
        let (n, s) = s.split_at(pos);
        (n.trim(), s.trim())
    } else {
        // No suffix, assume bytes
        (s.as_str(), "B")
    };

    let number: f64 = num_part
        .parse()
        .map_err(|_| format!("Invalid number: '{}'", num_part))?;

    let multiplier: usize = match suffix {
        "B" | "" => 1,
        "KB" | "K" => 1024,
        "MB" | "M" => 1024 * 1024,
        "GB" | "G" => 1024 * 1024 * 1024,
        _ => {
            return Err(format!(
                "Unknown size suffix: '{}'. Use B, KB, MB, or GB",
                suffix
            ))
        }
    };

    Ok((number * multiplier as f64) as usize)
}

/// Format bytes as human-readable size string
fn format_size(bytes: usize) -> String {
    const GB: usize = 1024 * 1024 * 1024;
    const MB: usize = 1024 * 1024;
    const KB: usize = 1024;

    if bytes >= GB && bytes.is_multiple_of(GB) {
        format!("{}GB", bytes / GB)
    } else if bytes >= MB && bytes.is_multiple_of(MB) {
        format!("{}MB", bytes / MB)
    } else if bytes >= KB && bytes.is_multiple_of(KB) {
        format!("{}KB", bytes / KB)
    } else if bytes >= GB {
        format!("{:.1}GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1}MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.1}KB", bytes as f64 / KB as f64)
    } else {
        format!("{}B", bytes)
    }
}

/// Load TLS configuration from certificate and key files
fn load_rustls_config(
    cert_path: &str,
    key_path: &str,
) -> Result<axum_server::tls_rustls::RustlsConfig, Box<dyn std::error::Error>> {
    use std::io::BufReader;

    // Install ring crypto provider (required for rustls 0.23+ with no-provider feature)
    let _ = rustls::crypto::ring::default_provider().install_default();

    // Read certificate file
    let cert_file = std::fs::File::open(cert_path)
        .map_err(|e| format!("Failed to open certificate file '{}': {}", cert_path, e))?;
    let mut cert_reader = BufReader::new(cert_file);
    let certs: Vec<_> = rustls_pemfile::certs(&mut cert_reader)
        .filter_map(|r| r.ok())
        .collect();

    if certs.is_empty() {
        return Err(format!("No valid certificates found in '{}'", cert_path).into());
    }

    // BUG #8 fix: Read key file once into memory, then try different formats
    // This avoids reopening the file and potential handle leaks on Windows
    let key_contents = std::fs::read(key_path)
        .map_err(|e| format!("Failed to read key file '{}': {}", key_path, e))?;

    // Try PKCS8 format first
    let pkcs8_keys: Vec<_> = rustls_pemfile::pkcs8_private_keys(&mut key_contents.as_slice())
        .filter_map(|r| r.ok())
        .collect();

    let key_der: Vec<u8> = if !pkcs8_keys.is_empty() {
        // BUG #14 fix: Use if-let instead of unwrap (can't fail since we checked is_empty)
        if let Some(key) = pkcs8_keys.into_iter().next() {
            key.secret_pkcs8_der().to_vec()
        } else {
            return Err(format!("No valid PKCS8 keys in '{}'", key_path).into());
        }
    } else {
        // Try RSA key format (reuse the same bytes in memory)
        let rsa_keys: Vec<_> = rustls_pemfile::rsa_private_keys(&mut key_contents.as_slice())
            .filter_map(|r| r.ok())
            .collect();

        if rsa_keys.is_empty() {
            return Err(format!("No valid private keys found in '{}'", key_path).into());
        }
        // BUG #14 fix: Use if-let instead of unwrap
        if let Some(key) = rsa_keys.into_iter().next() {
            key.secret_pkcs1_der().to_vec()
        } else {
            return Err(format!("No valid RSA keys in '{}'", key_path).into());
        }
    };

    // Build rustls config
    let config = axum_server::tls_rustls::RustlsConfig::from_der(
        certs.into_iter().map(|c| c.to_vec()).collect(),
        key_der,
    );

    // Block on the async config creation
    tokio::task::block_in_place(|| tokio::runtime::Handle::current().block_on(config))
        .map_err(|e| format!("Failed to create TLS config: {}", e).into())
}

/// Load configuration from environment or config file
/// Priority: CLI args (via env vars) > config file > defaults
pub fn load_config() -> Result<Config, Box<dyn std::error::Error>> {
    let config_path = std::env::var("MCP_CONFIG").unwrap_or_else(|_| "config.toml".to_string());

    let mut config = if std::path::Path::new(&config_path).exists() {
        let content = std::fs::read_to_string(&config_path)?;
        // Normalize Windows CRLF to LF for TOML parsing
        let content = content.replace("\r\n", "\n");
        let toml_config: TomlConfig =
            toml::from_str(&content).map_err(|e| format!("Failed to parse config: {}", e))?;

        // Parse max_body_size if provided, otherwise use default
        let max_body_size = match &toml_config.server.max_body_size {
            Some(size_str) => parse_size(size_str)
                .map_err(|e| format!("Invalid max_body_size '{}': {}", size_str, e))?,
            None => DEFAULT_MAX_BODY_SIZE,
        };

        Config {
            host: toml_config.server.host,
            port: toml_config.server.port,
            database_path: PathBuf::from(toml_config.database.path),
            max_body_size,
            require_api_key: toml_config.security.require_api_key,
            api_key_cache_ttl: toml_config.security.api_key_cache_ttl,
            tls_enabled: toml_config.tls.enabled,
            tls_cert_file: toml_config.tls.cert_file,
            tls_key_file: toml_config.tls.key_file,
            tool_timeout_secs: toml_config.server.tool_timeout_secs,
        }
    } else {
        // Check for IRONBASE_PATH env var
        let db_path =
            std::env::var("IRONBASE_PATH").unwrap_or_else(|_| "ironbase_data.mlite".to_string());
        Config {
            host: "0.0.0.0".to_string(),
            port: 8080,
            database_path: PathBuf::from(db_path),
            max_body_size: DEFAULT_MAX_BODY_SIZE,
            require_api_key: false,
            api_key_cache_ttl: 60,
            tls_enabled: false,
            tls_cert_file: None,
            tls_key_file: None,
            tool_timeout_secs: DEFAULT_TOOL_TIMEOUT_SECS,
        }
    };

    // CLI overrides via environment variables (set by main.rs from CLI args)
    if let Ok(port) = std::env::var("MCP_PORT") {
        if let Ok(p) = port.parse::<u16>() {
            config.port = p;
        }
    }
    if let Ok(host) = std::env::var("MCP_HOST") {
        config.host = host;
    }
    if let Ok(db_path) = std::env::var("IRONBASE_PATH") {
        config.database_path = PathBuf::from(db_path);
    }

    Ok(config)
}

#[derive(Debug, serde::Deserialize)]
struct TomlConfig {
    server: ServerConfig,
    database: DatabaseConfig,
    #[serde(default)]
    security: SecurityConfig,
    #[serde(default)]
    tls: TlsConfig,
}

#[derive(Debug, serde::Deserialize)]
struct ServerConfig {
    host: String,
    port: u16,
    /// Max body size in human-readable format: "1GB", "500MB", "10KB"
    #[serde(default)]
    max_body_size: Option<String>,
    /// Timeout for long-running tool operations in seconds (default: 300)
    #[serde(default = "default_tool_timeout")]
    tool_timeout_secs: u64,
}

fn default_tool_timeout() -> u64 {
    DEFAULT_TOOL_TIMEOUT_SECS
}

#[derive(Debug, serde::Deserialize)]
struct DatabaseConfig {
    path: String,
}

#[derive(Debug, serde::Deserialize, Default)]
struct SecurityConfig {
    /// If true, API key is required for all tool calls
    #[serde(default)]
    require_api_key: bool,
    /// Cache TTL for API keys in seconds (default: 60)
    #[serde(default = "default_cache_ttl")]
    api_key_cache_ttl: u64,
}

fn default_cache_ttl() -> u64 {
    60
}

#[derive(Debug, serde::Deserialize, Default)]
struct TlsConfig {
    /// If true, server uses HTTPS instead of HTTP
    #[serde(default)]
    enabled: bool,
    /// Path to TLS certificate file (PEM format)
    #[serde(default)]
    cert_file: Option<String>,
    /// Path to TLS private key file (PEM format)
    #[serde(default)]
    key_file: Option<String>,
}

/// Run HTTP server with default signal-based shutdown
pub async fn run_http_server() {
    run_http_server_internal(None).await;
}

/// Run HTTP server with an external shutdown receiver (used by Windows Service)
#[cfg(windows)]
pub async fn run_http_server_with_shutdown(shutdown_rx: std::sync::mpsc::Receiver<()>) {
    run_http_server_internal(Some(shutdown_rx)).await;
}

async fn run_http_server_internal(
    #[allow(unused_variables)] external_shutdown: Option<std::sync::mpsc::Receiver<()>>,
) {
    use axum::{
        extract::{DefaultBodyLimit, State},
        http::StatusCode,
        response::{IntoResponse, Response},
        routing::{get, post},
        Json, Router,
    };
    use tokio::net::TcpListener;
    use tracing::info;
    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::util::SubscriberInitExt;

    // Initialize tracing with dual output (stderr + file)
    // File logs go to ./logs/mcp-server.YYYY-MM-DD.log with daily rotation
    let env_filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));

    // Create logs directory if it doesn't exist
    let log_dir = std::path::Path::new("./logs");
    if !log_dir.exists() {
        if let Err(e) = std::fs::create_dir_all(log_dir) {
            eprintln!("Warning: Failed to create log directory: {}", e);
        }
    }

    // File appender with daily rotation
    let file_appender = tracing_appender::rolling::daily("./logs", "mcp-server.log");
    let (non_blocking_file, _guard) = tracing_appender::non_blocking(file_appender);

    // Dual output: stderr (with colors) + file (without colors)
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
    // (moved to static to prevent dropping)
    static LOG_GUARD: std::sync::OnceLock<tracing_appender::non_blocking::WorkerGuard> =
        std::sync::OnceLock::new();
    let _ = LOG_GUARD.set(_guard);

    info!("Starting MCP IronBase Server v{} (HTTP mode)", VERSION);

    // Log initial memory stats
    crate::monitoring::log_memory_stats();

    // Load configuration
    let config = match load_config() {
        Ok(c) => c,
        Err(e) => {
            tracing::error!("Failed to load configuration: {}", e);
            std::process::exit(1);
        }
    };
    let host = config.host.clone();
    let port = config.port;
    let max_body_size = config.max_body_size;

    // Initialize IronBase adapter
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

    // Initialize listener configuration in database
    {
        use crate::listener::ListenerManager;
        let listener_manager = ListenerManager::new(adapter.clone());
        if let Err(e) = listener_manager.init_default(&host, port, config.tls_enabled) {
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
    ));

    let app_state = Arc::new(HttpAppState {
        service,
        initialized: std::sync::atomic::AtomicBool::new(false),
        tool_timeout: std::time::Duration::from_secs(config.tool_timeout_secs),
    });

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

        // RAW request logging with trace ID
        let body_str = String::from_utf8_lossy(&body);
        tracing::debug!(trace_id = %trace_id, remote = %remote_addr, ">>> MCP REQUEST: {}", body_str);

        // Parse JSON
        let request: McpRequest = match serde_json::from_slice(&body) {
            Ok(req) => req,
            Err(e) => {
                let elapsed = request_start.elapsed();
                let error_response = format!("Failed to parse the request body as JSON: {}", e);
                tracing::error!(trace_id = %trace_id, elapsed_ms = elapsed.as_millis(), "<<< MCP PARSE ERROR: {}", error_response);
                return (StatusCode::BAD_REQUEST, error_response).into_response();
            }
        };

        // Extract API key from Authorization header or JSON params
        let api_key = extract_api_key(&headers, &request.params);

        // Clone state for spawn_blocking closure
        let state_clone = state.clone();
        let tool_timeout = state.tool_timeout;
        let request_id = request.id.clone();
        let request_method = request.method.clone();
        let trace_id_clone = trace_id.clone();

        // Log incoming request with method
        tracing::info!(trace_id = %trace_id, method = %request_method, remote = %remote_addr, "Request started");

        // Run potentially blocking operations in spawn_blocking with timeout
        // This prevents long-running operations (like index creation) from blocking the async runtime
        let result = tokio::time::timeout(tool_timeout, async move {
            tokio::task::spawn_blocking(move || {
                handle_request(
                    &request,
                    &state_clone.service,
                    &state_clone.initialized,
                    api_key.as_deref(),
                    Some(remote_addr),
                )
            })
            .await
        })
        .await;

        let elapsed = request_start.elapsed();

        match result {
            Ok(Ok(Some(response))) => {
                // Success - normal response
                tracing::info!(
                    trace_id = %trace_id_clone,
                    method = %request_method,
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
                    elapsed_ms = elapsed.as_millis(),
                    status = "notification",
                    "Request completed (no response)"
                );
                StatusCode::NO_CONTENT.into_response()
            }
            Ok(Err(join_error)) => {
                // spawn_blocking task panicked
                let error_msg = format!("Internal error: task panicked: {}", join_error);
                tracing::error!(
                    trace_id = %trace_id_clone,
                    method = %request_method,
                    elapsed_ms = elapsed.as_millis(),
                    status = "panic",
                    "<<< MCP ERROR: {}", error_msg
                );
                let error_response = create_error_response(-32603, &error_msg, request_id);
                (StatusCode::INTERNAL_SERVER_ERROR, Json(error_response)).into_response()
            }
            Err(_timeout_elapsed) => {
                // Timeout - operation took too long
                let timeout_secs = tool_timeout.as_secs();
                let error_msg = format!(
                    "Operation timed out after {} seconds. Method: '{}'. \
                    Solutions: 1) Add 'limit' to your query, 2) Use an indexed field in your filter, \
                    3) Increase tool_timeout_secs in config.toml.",
                    timeout_secs, request_method
                );
                tracing::error!(
                    trace_id = %trace_id_clone,
                    method = %request_method,
                    elapsed_ms = elapsed.as_millis(),
                    status = "timeout",
                    "<<< MCP TIMEOUT: {}", error_msg
                );
                let error_response = create_error_response(-32001, &error_msg, request_id);
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

    let addr = format!("{}:{}", host, port);

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

    // Run server - TLS or plain HTTP based on config
    if config.tls_enabled {
        // TLS mode with axum-server - validate config first
        let cert_file = match config.tls_cert_file.as_ref() {
            Some(path) => path,
            None => {
                tracing::error!("TLS enabled but tls.cert_file not set in config");
                std::process::exit(1);
            }
        };
        let key_file = match config.tls_key_file.as_ref() {
            Some(path) => path,
            None => {
                tracing::error!("TLS enabled but tls.key_file not set in config");
                std::process::exit(1);
            }
        };

        let rustls_config = match load_rustls_config(cert_file, key_file) {
            Ok(c) => c,
            Err(e) => {
                tracing::error!("Failed to load TLS config: {}", e);
                std::process::exit(1);
            }
        };

        info!(
            "Server listening on https://{} (TLS enabled, max body size: {})",
            addr,
            format_size(max_body_size)
        );

        // Use Handle for graceful shutdown with axum-server
        let handle = axum_server::Handle::new();
        let shutdown_handle = handle.clone();

        // Spawn a task to wait for shutdown signal
        tokio::spawn(async move {
            shutdown_future.await;
            shutdown_handle.graceful_shutdown(Some(std::time::Duration::from_secs(10)));
        });

        let socket_addr = match addr.parse() {
            Ok(a) => a,
            Err(e) => {
                tracing::error!("Invalid address '{}': {}", addr, e);
                std::process::exit(1);
            }
        };

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
                tracing::error!("Failed to bind to {}: {}", addr, e);
                std::process::exit(1);
            }
        };

        info!(
            "Server listening on http://{} (max body size: {})",
            addr,
            format_size(max_body_size)
        );

        // Use into_make_service_with_connect_info to enable ConnectInfo extraction
        let app_service = app.into_make_service_with_connect_info::<std::net::SocketAddr>();
        if let Err(e) = axum::serve(listener, app_service)
            .with_graceful_shutdown(shutdown_future)
            .await
        {
            tracing::error!("Server error: {}", e);
        }
    }

    // Graceful shutdown: checkpoint database
    info!("Shutting down gracefully...");
    if let Err(e) = adapter.checkpoint() {
        tracing::error!("Error checkpointing database: {}", e);
    }
    info!("Server stopped");
}

struct HttpAppState {
    /// Central service layer for tool execution
    service: Arc<crate::IronBaseService>,
    /// MCP lifecycle state: track if initialize has been called
    /// Per spec: "The initialization phase MUST be the first interaction"
    initialized: std::sync::atomic::AtomicBool,
    /// Timeout for long-running tool operations
    tool_timeout: std::time::Duration,
}

// MCP Request/Response types (duplicated from main.rs for lib independence)

#[derive(Debug, serde::Deserialize)]
struct McpRequest {
    #[serde(default)]
    #[allow(dead_code)]
    jsonrpc: Option<String>,
    #[serde(default)]
    id: Option<serde_json::Value>,
    method: String,
    #[serde(default)]
    params: serde_json::Value,
}

#[derive(Debug, serde::Serialize)]
#[serde(untagged)]
enum McpResponse {
    Success {
        jsonrpc: String,
        id: serde_json::Value,
        result: serde_json::Value,
    },
    Error {
        jsonrpc: String,
        id: serde_json::Value,
        error: McpErrorResponse,
    },
}

#[derive(Debug, serde::Serialize)]
struct McpErrorResponse {
    code: i32,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<serde_json::Value>,
}

#[derive(Debug, serde::Deserialize)]
struct ToolsCallParams {
    name: String,
    #[serde(default)]
    arguments: Option<serde_json::Value>,
}

#[derive(Debug, serde::Deserialize)]
struct PromptsGetParams {
    name: String,
    #[serde(default)]
    arguments: Option<serde_json::Value>,
}

#[derive(Debug, serde::Serialize)]
struct InitializeResult {
    #[serde(rename = "protocolVersion")]
    protocol_version: String,
    capabilities: Capabilities,
    #[serde(rename = "serverInfo")]
    server_info: ServerInfo,
}

#[derive(Debug, serde::Serialize)]
struct Capabilities {
    tools: serde_json::Value,
    prompts: serde_json::Value,
    resources: serde_json::Value,
}

#[derive(Debug, serde::Serialize)]
struct ServerInfo {
    name: String,
    version: String,
}

/// Handle MCP request using the service layer
///
/// This function handles MCP protocol routing and delegates tool execution
/// to the IronBaseService for authentication, authorization, and dispatch.
fn handle_request(
    request: &McpRequest,
    service: &crate::IronBaseService,
    initialized: &std::sync::atomic::AtomicBool,
    api_key: Option<&str>,
    remote_addr: Option<std::net::SocketAddr>,
) -> Option<McpResponse> {
    use crate::{
        get_prompt_content, get_prompts_list, get_resources_list, get_tools_list_filtered,
        read_resource, ServiceContext, ToolRequest,
    };
    use std::sync::atomic::Ordering;

    let is_notification = request.id.is_none() || matches!(&request.id, Some(v) if v.is_null());

    // MCP lifecycle enforcement: only allow initialize and ping before initialization
    // Per spec: "The initialization phase MUST be the first interaction"
    if !initialized.load(Ordering::SeqCst)
        && request.method != "initialize"
        && request.method != "ping"
        && !request.method.starts_with("notifications/")
    {
        return Some(create_error_response(
            -32002, // Server not initialized (custom error code)
            "Server not initialized. Call 'initialize' first.",
            request.id.clone(),
        ));
    }

    match request.method.as_str() {
        "initialize" => {
            initialized.store(true, Ordering::SeqCst);
            Some(create_success_response(
                serde_json::to_value(InitializeResult {
                    protocol_version: "2025-06-18".to_string(),
                    capabilities: Capabilities {
                        tools: serde_json::json!({"listChanged": false}),
                        prompts: serde_json::json!({"listChanged": false}),
                        resources: serde_json::json!({"subscribe": false, "listChanged": true}),
                    },
                    server_info: ServerInfo {
                        name: "ironbase-mcp".to_string(),
                        version: VERSION.to_string(),
                    },
                })
                .unwrap(),
                request.id.clone(),
            ))
        }

        "initialized" | "notifications/initialized" => None,

        "ping" => Some(create_success_response(
            serde_json::json!({}),
            request.id.clone(),
        )),

        "notifications/cancelled" => None,

        "tools/list" => {
            // SECURITY FIX #14: Filter admin tools for non-localhost callers
            let is_localhost = remote_addr
                .map(|addr| crate::InterfaceType::from_socket_addr(addr) == crate::InterfaceType::Localhost)
                .unwrap_or(false);
            Some(create_success_response(
                get_tools_list_filtered(is_localhost),
                request.id.clone(),
            ))
        }

        "tools/call" => {
            let params: ToolsCallParams = match serde_json::from_value(request.params.clone()) {
                Ok(p) => p,
                Err(e) => {
                    return Some(create_error_response(
                        -32602,
                        &format!("Invalid params: {}", e),
                        request.id.clone(),
                    ));
                }
            };

            let arguments = params.arguments.unwrap_or_else(|| serde_json::json!({}));

            // Create service context
            let caller = CallerContext::new(remote_addr, api_key.map(|s| s.to_string()));
            let ctx = ServiceContext::new(caller, initialized.load(Ordering::SeqCst));

            // Create tool request
            let tool_request = ToolRequest::new(&params.name, arguments);

            // Execute via service layer (handles auth, ACL, dispatch)
            match service.execute_tool(&ctx, &tool_request) {
                crate::ToolResult::Success(result) => {
                    let response = serde_json::json!({
                        "content": [{
                            "type": "text",
                            "text": serde_json::to_string_pretty(&result).unwrap_or_else(|_| "{}".to_string())
                        }]
                    });
                    Some(create_success_response(response, request.id.clone()))
                }
                crate::ToolResult::Error { code, message } => {
                    let response = serde_json::json!({
                        "content": [{
                            "type": "text",
                            "text": format!("Error: {}", message)
                        }],
                        "isError": true
                    });
                    // Use code for JSON-RPC error if it's a standard error
                    if code == -32602 || code == -32601 {
                        Some(create_error_response(code, &message, request.id.clone()))
                    } else {
                        Some(create_success_response(response, request.id.clone()))
                    }
                }
                crate::ToolResult::AccessDenied(msg) => {
                    let response = serde_json::json!({
                        "content": [{
                            "type": "text",
                            "text": msg
                        }],
                        "isError": true
                    });
                    Some(create_success_response(response, request.id.clone()))
                }
            }
        }

        "prompts/list" => Some(create_success_response(
            get_prompts_list(),
            request.id.clone(),
        )),

        "prompts/get" => {
            let params: PromptsGetParams = match serde_json::from_value(request.params.clone()) {
                Ok(p) => p,
                Err(e) => {
                    return Some(create_error_response(
                        -32602,
                        &format!("Invalid params: {}", e),
                        request.id.clone(),
                    ));
                }
            };

            let arguments = params.arguments.unwrap_or_else(|| serde_json::json!({}));

            match get_prompt_content(&params.name, &arguments) {
                Some(content) => Some(create_success_response(content, request.id.clone())),
                None => Some(create_error_response(
                    -32602,
                    &format!("Prompt '{}' not found", params.name),
                    request.id.clone(),
                )),
            }
        }

        "resources/list" => Some(create_success_response(
            get_resources_list(service.adapter()),
            request.id.clone(),
        )),

        "resources/read" => {
            #[derive(serde::Deserialize)]
            struct ResourcesReadParams {
                uri: String,
            }

            let params: ResourcesReadParams = match serde_json::from_value(request.params.clone()) {
                Ok(p) => p,
                Err(e) => {
                    return Some(create_error_response(
                        -32602,
                        &format!("Invalid params: {}", e),
                        request.id.clone(),
                    ));
                }
            };

            match read_resource(service.adapter(), &params.uri) {
                Ok(content) => Some(create_success_response(content, request.id.clone())),
                Err(e) => Some(create_error_response(-32602, &e, request.id.clone())),
            }
        }

        _ if is_notification => None,

        _ => Some(create_error_response(
            -32601,
            &format!("Method not found: {}", request.method),
            request.id.clone(),
        )),
    }
}

fn create_success_response(
    result: serde_json::Value,
    id: Option<serde_json::Value>,
) -> McpResponse {
    McpResponse::Success {
        jsonrpc: "2.0".to_string(),
        id: id.unwrap_or(serde_json::Value::Null),
        result,
    }
}

fn create_error_response(code: i32, message: &str, id: Option<serde_json::Value>) -> McpResponse {
    McpResponse::Error {
        jsonrpc: "2.0".to_string(),
        id: id.unwrap_or(serde_json::Value::Null),
        error: McpErrorResponse {
            code,
            message: message.to_string(),
            data: None,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_size_bytes() {
        assert_eq!(parse_size("100").unwrap(), 100);
        assert_eq!(parse_size("100B").unwrap(), 100);
        assert_eq!(parse_size("100b").unwrap(), 100);
    }

    #[test]
    fn test_parse_size_kilobytes() {
        assert_eq!(parse_size("1KB").unwrap(), 1024);
        assert_eq!(parse_size("1kb").unwrap(), 1024);
        assert_eq!(parse_size("1K").unwrap(), 1024);
        assert_eq!(parse_size("10KB").unwrap(), 10 * 1024);
    }

    #[test]
    fn test_parse_size_megabytes() {
        assert_eq!(parse_size("1MB").unwrap(), 1024 * 1024);
        assert_eq!(parse_size("1mb").unwrap(), 1024 * 1024);
        assert_eq!(parse_size("1M").unwrap(), 1024 * 1024);
        assert_eq!(parse_size("500MB").unwrap(), 500 * 1024 * 1024);
    }

    #[test]
    fn test_parse_size_gigabytes() {
        assert_eq!(parse_size("1GB").unwrap(), 1024 * 1024 * 1024);
        assert_eq!(parse_size("1gb").unwrap(), 1024 * 1024 * 1024);
        assert_eq!(parse_size("1G").unwrap(), 1024 * 1024 * 1024);
        assert_eq!(parse_size("2GB").unwrap(), 2 * 1024 * 1024 * 1024);
    }

    #[test]
    fn test_parse_size_with_spaces() {
        assert_eq!(parse_size("  1GB  ").unwrap(), 1024 * 1024 * 1024);
        assert_eq!(parse_size("1 GB").unwrap(), 1024 * 1024 * 1024);
    }

    #[test]
    fn test_parse_size_fractional() {
        assert_eq!(
            parse_size("1.5GB").unwrap(),
            (1.5 * 1024.0 * 1024.0 * 1024.0) as usize
        );
        assert_eq!(
            parse_size("0.5MB").unwrap(),
            (0.5 * 1024.0 * 1024.0) as usize
        );
    }

    #[test]
    fn test_parse_size_invalid() {
        assert!(parse_size("abc").is_err());
        assert!(parse_size("1TB").is_err()); // TB not supported
        assert!(parse_size("").is_err());
    }

    #[test]
    fn test_format_size() {
        assert_eq!(format_size(1024), "1KB");
        assert_eq!(format_size(1024 * 1024), "1MB");
        assert_eq!(format_size(1024 * 1024 * 1024), "1GB");
        assert_eq!(format_size(500 * 1024 * 1024), "500MB");
        assert_eq!(format_size(100), "100B");
    }

    #[test]
    fn test_format_size_fractional() {
        // 1.5 GB = 1536 MB (exact), so it shows as MB
        let size = (1.5 * 1024.0 * 1024.0 * 1024.0) as usize;
        assert_eq!(format_size(size), "1536MB");

        // Non-exact values show decimals
        let size2 = 1024 * 1024 * 1024 + 512 * 1024 * 1024; // 1.5 GB
        assert_eq!(format_size(size2), "1536MB");
    }
}
