//! IronBase MCP Bridge
//!
//! STDIO to HTTP/HTTPS bridge for MCP IronBase Server.
//! Compatible with Claude Desktop, ChatGPT Desktop, VS Code Copilot, and other MCP clients.
//!
//! Features:
//! - Connection pooling with keep-alive
//! - Self-signed certificate support (--insecure)
//! - Graceful shutdown (SIGINT/SIGTERM)
//! - Health check with retry logic
//! - JSON-RPC 2.0 batch request support
//! - Cross-platform (Windows, Linux, macOS)

use anyhow::{Context, Result};
use clap::Parser;
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, BufReader};

// BUG #5 fix: Limit batch request size to prevent DoS
const MAX_BATCH_SIZE: usize = 1000;

// BUG #6 fix: Limit response body size to prevent memory exhaustion
const MAX_RESPONSE_SIZE: usize = 10 * 1024 * 1024; // 10 MB

// BUG #10 fix: Maximum backoff time for health check retries
const MAX_HEALTH_BACKOFF_MS: u64 = 5000;

// BUG #4 fix: Track stdout health for graceful exit
static STDOUT_BROKEN: AtomicBool = AtomicBool::new(false);

/// Parse boolean from env var - accepts "1", "true", "yes" (case-insensitive)
fn parse_bool_flexible(s: &str) -> Result<bool, String> {
    match s.to_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Ok(true),
        "0" | "false" | "no" | "off" | "" => Ok(false),
        _ => Err(format!(
            "Invalid boolean value: '{}'. Use true/false/1/0",
            s
        )),
    }
}

// ============================================================================
// CLI Arguments
// ============================================================================

#[derive(Parser, Debug)]
#[command(name = "ironbase-bridge")]
#[command(version, about = "STDIO to HTTP/HTTPS bridge for MCP IronBase Server")]
struct Args {
    /// Server URL
    #[arg(
        short,
        long,
        env = "MCP_SERVER_URL",
        default_value = "http://localhost:8080/mcp"
    )]
    server: String,

    /// API key for authentication
    #[arg(short = 'k', long, env = "IRONBASE_API_KEY")]
    api_key: Option<String>,

    /// Accept invalid/self-signed certificates (env: 1/true/yes)
    #[arg(long, env = "MCP_INSECURE", value_parser = parse_bool_flexible, default_value = "false")]
    insecure: bool,

    /// Enable debug logging (env: 1/true/yes)
    #[arg(short, long, env = "MCP_DEBUG", value_parser = parse_bool_flexible, default_value = "false")]
    debug: bool,

    /// Health check retries before giving up (0 = skip health check)
    #[arg(long, default_value = "3")]
    health_retries: u32,
}

// ============================================================================
// JSON-RPC 2.0 Types
// ============================================================================

#[derive(Debug, Clone, Deserialize, Serialize)]
struct JsonRpcRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    jsonrpc: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<serde_json::Value>,
    method: String,
    #[serde(default)]
    params: serde_json::Value,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct JsonRpcResponse {
    jsonrpc: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<JsonRpcError>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct JsonRpcError {
    code: i32,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<serde_json::Value>,
}

#[derive(Debug)]
enum JsonRpcInput {
    Single(JsonRpcRequest),
    Batch {
        requests: Vec<JsonRpcRequest>,
        errors: Vec<JsonRpcResponse>,
    },
}

// ============================================================================
// Helper Functions
// ============================================================================

/// Check if request is a notification (no id)
fn is_notification(request: &JsonRpcRequest) -> bool {
    request.id.is_none()
}

/// Create an error response
fn make_error_response(id: Option<serde_json::Value>, code: i32, message: &str) -> JsonRpcResponse {
    JsonRpcResponse {
        jsonrpc: "2.0".to_string(),
        id,
        result: None,
        error: Some(JsonRpcError {
            code,
            message: message.to_string(),
            data: None,
        }),
    }
}

#[derive(Debug)]
enum ParseInputError {
    ParseError(String),
    InvalidRequest {
        id: Option<serde_json::Value>,
        message: String,
    },
}

impl ParseInputError {
    fn code(&self) -> i32 {
        match self {
            ParseInputError::ParseError(_) => -32700,
            ParseInputError::InvalidRequest { .. } => -32600,
        }
    }

    fn message(&self) -> String {
        match self {
            ParseInputError::ParseError(msg) => msg.clone(),
            ParseInputError::InvalidRequest { message, .. } => message.clone(),
        }
    }

    fn id(&self) -> Option<serde_json::Value> {
        match self {
            ParseInputError::InvalidRequest { id, .. } => id.clone(),
            _ => None,
        }
    }
}

fn extract_valid_id(value: &serde_json::Value) -> Option<serde_json::Value> {
    match value {
        serde_json::Value::Object(map) => match map.get("id") {
            Some(serde_json::Value::String(_)) => map.get("id").cloned(),
            Some(serde_json::Value::Number(_)) => map.get("id").cloned(),
            _ => None,
        },
        _ => None,
    }
}

/// Validate JSON-RPC version field
/// BUG #12 fix: jsonrpc field must be exactly "2.0" if present
fn validate_jsonrpc_version(request: &JsonRpcRequest) -> Result<()> {
    if let Some(version) = &request.jsonrpc {
        if version != "2.0" {
            anyhow::bail!("Invalid JSON-RPC version '{}', expected '2.0'", version);
        }
    }
    Ok(())
}

/// Parse input as single request or batch
/// BUG #5 fix: Limit batch size to MAX_BATCH_SIZE
/// BUG #11 fix: Reject empty batch
/// BUG #12 fix: Validate jsonrpc version
fn parse_input(line: &str) -> Result<JsonRpcInput, ParseInputError> {
    let trimmed = line.trim();
    let value: serde_json::Value =
        serde_json::from_str(trimmed).map_err(|e| ParseInputError::ParseError(e.to_string()))?;

    let parse_request = |value: &serde_json::Value| -> Result<JsonRpcRequest, ParseInputError> {
        if !value.is_object() {
            return Err(ParseInputError::InvalidRequest {
                id: None,
                message: "Request must be an object".to_string(),
            });
        }

        let request: JsonRpcRequest = serde_json::from_value(value.clone()).map_err(|e| {
            ParseInputError::InvalidRequest {
                id: extract_valid_id(value),
                message: format!("Invalid request: {}", e),
            }
        })?;

        if let Some(serde_json::Value::Null) = &request.id {
            return Err(ParseInputError::InvalidRequest {
                id: None,
                message: "Invalid request: id must be omitted for notifications".to_string(),
            });
        }

        validate_jsonrpc_version(&request).map_err(|e| ParseInputError::InvalidRequest {
            id: request.id.clone(),
            message: e.to_string(),
        })?;

        Ok(request)
    };

    match value {
        serde_json::Value::Array(values) => {
            // BUG #11 fix: Empty batch is invalid per JSON-RPC 2.0 spec
            if values.is_empty() {
                return Err(ParseInputError::InvalidRequest {
                    id: None,
                    message: "Empty batch request is invalid".to_string(),
                });
            }

            // BUG #5 fix: Limit batch size to prevent DoS
            if values.len() > MAX_BATCH_SIZE {
                return Err(ParseInputError::InvalidRequest {
                    id: None,
                    message: format!(
                        "Batch size {} exceeds maximum allowed ({})",
                        values.len(),
                        MAX_BATCH_SIZE
                    ),
                });
            }

            let mut requests = Vec::new();
            let mut errors = Vec::new();

            for value in values {
                match parse_request(&value) {
                    Ok(req) => requests.push(req),
                    Err(err) => errors.push(make_error_response(err.id(), err.code(), &err.message())),
                }
            }

            Ok(JsonRpcInput::Batch { requests, errors })
        }
        serde_json::Value::Object(_) => {
            let request = parse_request(&value)?;
            Ok(JsonRpcInput::Single(request))
        }
        _ => Err(ParseInputError::InvalidRequest {
            id: None,
            message: "Request must be an object or batch array".to_string(),
        }),
    }
}

/// Write JSON response to stdout with proper flushing
/// CRITICAL: This is the ONLY function that writes to stdout
/// BUG #4 fix: Detect broken pipe and signal for graceful exit
fn write_response(json: &str) -> bool {
    let stdout = std::io::stdout();
    let mut handle = stdout.lock();

    if let Err(e) = writeln!(handle, "{}", json) {
        // Broken pipe (client disconnected) - signal exit
        if e.kind() == std::io::ErrorKind::BrokenPipe {
            STDOUT_BROKEN.store(true, Ordering::SeqCst);
            return false;
        }
        tracing::error!("Failed to write to stdout: {}", e);
        STDOUT_BROKEN.store(true, Ordering::SeqCst);
        return false;
    }

    if let Err(e) = handle.flush() {
        if e.kind() == std::io::ErrorKind::BrokenPipe {
            STDOUT_BROKEN.store(true, Ordering::SeqCst);
            return false;
        }
        tracing::error!("Failed to flush stdout: {}", e);
        STDOUT_BROKEN.store(true, Ordering::SeqCst);
        return false;
    }

    true
}

/// Setup panic handler to write to stderr, not stdout
fn setup_panic_handler() {
    std::panic::set_hook(Box::new(|panic_info| {
        eprintln!("PANIC: {}", panic_info);
    }));
}

/// Setup logging to stderr
fn setup_logging(args: &Args) {
    use tracing_subscriber::EnvFilter;

    let filter = if args.debug {
        EnvFilter::new("debug")
    } else {
        EnvFilter::new("warn")
    };

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .init();
}

// ============================================================================
// HTTP Client
// ============================================================================

/// Build HTTP client with connection pooling
/// BUG #8 fix: Better documentation and warnings for insecure mode
fn build_client(args: &Args) -> Result<reqwest::Client> {
    let mut builder = reqwest::Client::builder()
        .pool_max_idle_per_host(2)
        .pool_idle_timeout(Duration::from_secs(60))
        .timeout(Duration::from_secs(30))
        .use_rustls_tls();

    // BUG #8 fix: Enhanced warning for insecure mode
    // Self-signed cert support - USE WITH CAUTION
    if args.insecure {
        tracing::warn!("╔════════════════════════════════════════════════════════╗");
        tracing::warn!("║ WARNING: TLS CERTIFICATE VERIFICATION DISABLED         ║");
        tracing::warn!("║                                                        ║");
        tracing::warn!("║ The --insecure flag is enabled. This disables ALL      ║");
        tracing::warn!("║ certificate validation, making the connection          ║");
        tracing::warn!("║ vulnerable to man-in-the-middle (MITM) attacks.        ║");
        tracing::warn!("║                                                        ║");
        tracing::warn!("║ Only use this for:                                     ║");
        tracing::warn!("║   - Local development with self-signed certs           ║");
        tracing::warn!("║   - Testing environments                               ║");
        tracing::warn!("║                                                        ║");
        tracing::warn!("║ NEVER use --insecure in production!                    ║");
        tracing::warn!("╚════════════════════════════════════════════════════════╝");
        builder = builder.danger_accept_invalid_certs(true);
    }

    builder.build().context("Failed to build HTTP client")
}

/// Health check with retry logic
/// BUG #7 fix: Use proper URL parsing instead of string manipulation
/// BUG #10 fix: Cap backoff time to MAX_HEALTH_BACKOFF_MS
async fn wait_for_server(client: &reqwest::Client, url: &str, retries: u32) -> Result<()> {
    if retries == 0 {
        tracing::debug!("Health check disabled");
        return Ok(());
    }

    // BUG #7 fix: Proper URL construction
    let health_url = match url::Url::parse(url) {
        Ok(mut parsed) => {
            // Remove /mcp path if present and add /health
            let path = parsed.path().trim_end_matches('/');
            let new_path = if path.ends_with("/mcp") {
                format!("{}/health", path.trim_end_matches("/mcp"))
            } else {
                format!("{}/health", path)
            };
            parsed.set_path(&new_path);
            parsed.to_string()
        }
        Err(e) => {
            tracing::warn!("Failed to parse URL '{}': {}, using fallback", url, e);
            // Fallback to simple string replacement
            url.trim_end_matches("/mcp").to_string() + "/health"
        }
    };
    tracing::debug!("Health check URL: {}", health_url);

    for attempt in 1..=retries {
        match client.get(&health_url).send().await {
            Ok(resp) if resp.status().is_success() => {
                tracing::info!("Server reachable at {}", url);
                return Ok(());
            }
            Ok(resp) => {
                tracing::warn!(
                    "Health check attempt {}/{}: HTTP {}",
                    attempt,
                    retries,
                    resp.status()
                );
            }
            Err(e) => {
                tracing::warn!("Health check attempt {}/{}: {}", attempt, retries, e);
            }
        }

        if attempt < retries {
            // BUG #10 fix: Cap backoff to MAX_HEALTH_BACKOFF_MS
            let backoff_ms = std::cmp::min(500 * attempt as u64, MAX_HEALTH_BACKOFF_MS);
            tokio::time::sleep(Duration::from_millis(backoff_ms)).await;
        }
    }

    tracing::warn!(
        "Server not reachable after {} attempts, continuing anyway...",
        retries
    );
    Ok(())
}

/// Forward a single request to the server
/// BUG #6 fix: Limit response body size to prevent memory exhaustion
async fn forward_single(
    client: &reqwest::Client,
    args: &Args,
    request: &JsonRpcRequest,
) -> Result<Option<JsonRpcResponse>> {
    let mut req = client
        .post(&args.server)
        .header("Content-Type", "application/json")
        .json(request);

    // Add API key if configured
    if let Some(key) = &args.api_key {
        req = req.header("Authorization", format!("Bearer {}", key));
    }

    let response = req.send().await.context("HTTP request failed")?;

    // 204 No Content = notification acknowledged
    if response.status() == reqwest::StatusCode::NO_CONTENT {
        return Ok(None);
    }

    // BUG #6 fix: Check Content-Length before reading body
    if let Some(content_length) = response.content_length() {
        if content_length as usize > MAX_RESPONSE_SIZE {
            return Ok(Some(make_error_response(
                request.id.clone(),
                -32603,
                &format!(
                    "Response too large: {} bytes (max: {} bytes)",
                    content_length, MAX_RESPONSE_SIZE
                ),
            )));
        }
    }

    // Check for errors
    // BUG #2 fix: Handle error response body read failures properly
    if !response.status().is_success() {
        let status = response.status();
        let text = match response.text().await {
            Ok(t) => {
                // BUG #6 fix: Truncate error body if too large
                if t.len() > MAX_RESPONSE_SIZE {
                    format!("{}...<truncated>", &t[..1000])
                } else {
                    t
                }
            }
            Err(e) => {
                tracing::warn!("Failed to read error response body: {}", e);
                format!("<body read error: {}>", e)
            }
        };
        return Ok(Some(make_error_response(
            request.id.clone(),
            -32603,
            &format!("HTTP {}: {}", status, text),
        )));
    }

    // Empty body = notification acknowledged
    let text = response
        .text()
        .await
        .context("Failed to read response body")?;

    // BUG #6 fix: Check actual body size
    if text.len() > MAX_RESPONSE_SIZE {
        return Ok(Some(make_error_response(
            request.id.clone(),
            -32603,
            &format!(
                "Response body too large: {} bytes (max: {} bytes)",
                text.len(),
                MAX_RESPONSE_SIZE
            ),
        )));
    }

    if text.is_empty() {
        return Ok(None);
    }

    // Parse response
    let resp: JsonRpcResponse =
        serde_json::from_str(&text).context("Failed to parse JSON-RPC response")?;
    Ok(Some(resp))
}

/// Process a batch of requests
/// BUG #9 fix: Log notification forward errors
async fn process_batch(
    client: &reqwest::Client,
    args: &Args,
    requests: Vec<JsonRpcRequest>,
) -> Vec<JsonRpcResponse> {
    let mut responses = Vec::new();

    for request in requests {
        if is_notification(&request) {
            // BUG #9 fix: Log notification forward errors instead of ignoring
            if let Err(e) = forward_single(client, args, &request).await {
                tracing::warn!("Notification '{}' forward failed: {}", request.method, e);
            }
            continue;
        }

        match forward_single(client, args, &request).await {
            Ok(Some(resp)) => responses.push(resp),
            Ok(None) => {} // notification response
            Err(e) => {
                responses.push(make_error_response(
                    request.id.clone(),
                    -32603,
                    &e.to_string(),
                ));
            }
        }
    }

    responses
}

// ============================================================================
// Signal Handling
// ============================================================================

/// BUG #1 fix: Handle signal installation errors gracefully
#[cfg(unix)]
async fn shutdown_signal() {
    use tokio::signal::unix::{signal, SignalKind};

    let sigint = async {
        match signal(SignalKind::interrupt()) {
            Ok(mut s) => {
                s.recv().await;
            }
            Err(e) => {
                tracing::error!("Failed to setup SIGINT handler: {}", e);
                std::future::pending::<()>().await;
            }
        }
    };

    let sigterm = async {
        match signal(SignalKind::terminate()) {
            Ok(mut s) => {
                s.recv().await;
            }
            Err(e) => {
                tracing::error!("Failed to setup SIGTERM handler: {}", e);
                std::future::pending::<()>().await;
            }
        }
    };

    tokio::select! {
        _ = sigint => tracing::info!("Received SIGINT"),
        _ = sigterm => tracing::info!("Received SIGTERM"),
    }
}

/// BUG #1 fix: Handle signal installation errors gracefully
#[cfg(windows)]
async fn shutdown_signal() {
    match tokio::signal::ctrl_c().await {
        Ok(()) => tracing::info!("Received Ctrl+C"),
        Err(e) => {
            tracing::error!("Failed to setup Ctrl+C handler: {}", e);
            std::future::pending::<()>().await;
        }
    }
}

// ============================================================================
// Main Loop
// ============================================================================

/// BUG #3 fix: Helper to serialize JSON with proper error handling
fn serialize_response<T: serde::Serialize>(value: &T) -> Option<String> {
    match serde_json::to_string(value) {
        Ok(json) => Some(json),
        Err(e) => {
            // BUG #3 fix: Log serialization error and send error response
            tracing::error!("Failed to serialize response: {}", e);
            // Create a minimal fallback error response
            Some(r#"{"jsonrpc":"2.0","id":null,"error":{"code":-32603,"message":"Internal error: failed to serialize response"}}"#.to_string())
        }
    }
}

/// BUG #3 fix: Proper JSON serialization error handling
/// BUG #4 fix: Check STDOUT_BROKEN and propagate write errors
/// BUG #9 fix: Log notification forward errors
async fn process_line(client: &reqwest::Client, args: &Args, line: &str) -> bool {
    // BUG #4 fix: Check if stdout is already broken
    if STDOUT_BROKEN.load(Ordering::SeqCst) {
        return false;
    }

    let output = match parse_input(line) {
        Ok(JsonRpcInput::Single(request)) => {
            tracing::debug!(
                "Processing request: {} (id: {:?})",
                request.method,
                request.id
            );

            if is_notification(&request) {
                // BUG #9 fix: Log notification forward errors
                if let Err(e) = forward_single(client, args, &request).await {
                    tracing::warn!("Notification '{}' forward failed: {}", request.method, e);
                }
                return true;
            }

            match forward_single(client, args, &request).await {
                Ok(Some(resp)) => serialize_response(&resp),
                Ok(None) => return true, // notification response
                Err(e) => {
                    tracing::error!("Request failed: {}", e);
                    serialize_response(&make_error_response(request.id, -32603, &e.to_string()))
                }
            }
        }
        Ok(JsonRpcInput::Batch { requests, mut errors }) => {
            tracing::debug!("Processing batch of {} requests", requests.len());

            let mut responses = process_batch(client, args, requests).await;
            responses.append(&mut errors);
            if responses.is_empty() {
                return true; // all were notifications
            }
            serialize_response(&responses)
        }
        Err(e) => {
            tracing::error!("Parse error: {}", e.message());
            serialize_response(&make_error_response(
                e.id(),
                e.code(),
                &e.message(),
            ))
        }
    };

    if let Some(json) = output {
        // BUG #4 fix: Check write_response return value
        if !write_response(&json) {
            return false;
        }
    }
    true
}

#[tokio::main]
async fn main() -> Result<()> {
    // FIRST: Setup panic handler before anything else
    setup_panic_handler();

    // Parse CLI arguments
    let args = Args::parse();

    // Setup logging to stderr
    setup_logging(&args);

    tracing::info!("ironbase-bridge v{} starting", env!("CARGO_PKG_VERSION"));
    tracing::info!("Server: {}", args.server);
    if args.api_key.is_some() {
        tracing::info!("API key: configured");
    }

    // Build HTTP client with connection pooling
    let client = build_client(&args)?;

    // Health check with retry
    wait_for_server(&client, &args.server, args.health_retries).await?;

    // Setup stdin reader
    let stdin = tokio::io::stdin();
    let reader = BufReader::new(stdin);
    let mut lines = reader.lines();

    tracing::debug!("Entering main loop...");

    // Main loop with graceful shutdown
    // BUG #4 fix: Exit on stdout broken
    loop {
        // BUG #4 fix: Check stdout before each iteration
        if STDOUT_BROKEN.load(Ordering::SeqCst) {
            tracing::info!("stdout broken, exiting...");
            break;
        }

        tokio::select! {
            // Read next line from stdin
            line_result = lines.next_line() => {
                match line_result {
                    Ok(Some(line)) if !line.trim().is_empty() => {
                        // BUG #4 fix: Check process_line return value
                        if !process_line(&client, &args, &line).await {
                            tracing::info!("stdout broken during processing, exiting...");
                            break;
                        }
                    }
                    Ok(Some(_)) => continue, // empty line
                    Ok(None) => {
                        tracing::info!("EOF - exiting");
                        break;
                    }
                    Err(e) => {
                        tracing::error!("Read error: {}", e);
                        break;
                    }
                }
            }
            // Shutdown signal
            _ = shutdown_signal() => {
                tracing::info!("Shutting down gracefully...");
                break;
            }
        }
    }

    tracing::info!("Bridge stopped");
    Ok(())
}
