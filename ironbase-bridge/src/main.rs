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

/// Time to establish the TCP/TLS connection to the server
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// Characters of an oversized HTTP error body kept in the error message
const ERROR_BODY_PREVIEW_CHARS: usize = 1000;

/// Tools the admin key is injected into (the server requires it only there)
const ADMIN_TOOL_PREFIX: &str = "admin_";

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

    /// Admin key for privileged operations (injected into tools/call arguments)
    #[arg(long, env = "IRONBASE_ADMIN_KEY")]
    admin_key: Option<String>,

    /// Accept invalid/self-signed certificates (env: 1/true/yes)
    #[arg(long, env = "MCP_INSECURE", value_parser = parse_bool_flexible, default_value = "false")]
    insecure: bool,

    /// Enable debug logging (env: 1/true/yes)
    #[arg(short, long, env = "MCP_DEBUG", value_parser = parse_bool_flexible, default_value = "false")]
    debug: bool,

    /// Health check retries before giving up (0 = skip health check)
    #[arg(long, default_value = "3")]
    health_retries: u32,

    /// Seconds to wait for the server's answer to one request (0 = no
    /// limit). Generous by default: the server builds indexes without a
    /// time limit.
    #[arg(long, env = "MCP_TIMEOUT_SECS", default_value = "3600")]
    timeout_secs: u64,
}

// ============================================================================
// JSON-RPC 2.0 Types
// ============================================================================

/// A field that is present, `null` included (`#[serde(default)]` gives
/// `None` when it is missing)
fn present_value<'de, D>(
    deserializer: D,
) -> std::result::Result<Option<serde_json::Value>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    serde_json::Value::deserialize(deserializer).map(Some)
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct JsonRpcRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    jsonrpc: Option<String>,
    /// `"id": null` is `Some(Null)` (an invalid request), a missing id
    /// `None` (a notification): plain `Option` read both as `None`, so a
    /// null-id request was forwarded as a notification and never answered
    #[serde(
        default,
        deserialize_with = "present_value",
        skip_serializing_if = "Option::is_none"
    )]
    id: Option<serde_json::Value>,
    method: String,
    /// Absent params stay absent when forwarded: JSON-RPC 2.0 allows
    /// omitting them, not `null`
    #[serde(default, skip_serializing_if = "serde_json::Value::is_null")]
    params: serde_json::Value,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct JsonRpcResponse {
    jsonrpc: String,
    /// Always present: JSON-RPC 2.0 requires `"id": null` when the request's
    /// id could not be determined (parse error, invalid request)
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

        let request: JsonRpcRequest =
            serde_json::from_value(value.clone()).map_err(|e| ParseInputError::InvalidRequest {
                id: extract_valid_id(value),
                message: format!("Invalid request: {}", e),
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
                    Err(err) => {
                        errors.push(make_error_response(err.id(), err.code(), &err.message()))
                    }
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
        .connect_timeout(CONNECT_TIMEOUT)
        .use_rustls_tls();

    // Without a limit a server that never answers held the bridge (and the
    // MCP client waiting on it) forever
    if args.timeout_secs > 0 {
        builder = builder.timeout(Duration::from_secs(args.timeout_secs));
    }

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

/// Whether a request calls an admin tool (`tools/call` of an `admin_*` tool):
/// the only calls the admin key is injected into
fn is_admin_tool_call(request: &JsonRpcRequest) -> bool {
    request.method == "tools/call"
        && request
            .params
            .get("name")
            .and_then(|name| name.as_str())
            .is_some_and(|name| name.starts_with(ADMIN_TOOL_PREFIX))
}

/// Inject admin_key into the arguments of an admin tool call.
/// Only call this when `is_admin_tool_call` holds: injecting it everywhere
/// handed every tool call the admin key.
fn inject_admin_key(request: &JsonRpcRequest, admin_key: &str) -> JsonRpcRequest {
    let mut request = request.clone();
    if let serde_json::Value::Object(ref mut params) = request.params {
        if let Some(serde_json::Value::Object(ref mut arguments)) = params.get_mut("arguments") {
            // Only inject if not already present (explicit value takes priority)
            if !arguments.contains_key("admin_key") {
                arguments.insert(
                    "admin_key".to_string(),
                    serde_json::Value::String(admin_key.to_string()),
                );
            }
        }
    }

    request
}

/// Forward a single request to the server
/// BUG #6 fix: Limit response body size to prevent memory exhaustion
async fn forward_single(
    client: &reqwest::Client,
    args: &Args,
    request: &JsonRpcRequest,
) -> Result<Option<JsonRpcResponse>> {
    // Inject admin_key into admin tool calls if configured
    let owned_request;
    let request_ref = match &args.admin_key {
        Some(admin_key) if is_admin_tool_call(request) => {
            owned_request = inject_admin_key(request, admin_key);
            &owned_request
        }
        _ => request,
    };

    let mut req = client
        .post(&args.server)
        .header("Content-Type", "application/json")
        .json(request_ref);

    // Add API key if configured
    if let Some(key) = &args.api_key {
        req = req.header("Authorization", format!("Bearer {}", key));
    }

    let response = req.send().await.context("HTTP request failed")?;

    // 204 No Content = notification acknowledged
    if response.status() == reqwest::StatusCode::NO_CONTENT {
        return Ok(no_answer(request));
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
        let text = match read_body_limited(response, MAX_RESPONSE_SIZE).await {
            Ok(Body::Complete(bytes)) => String::from_utf8_lossy(&bytes).into_owned(),
            // BUG #6 fix: Truncate error body if too large
            Ok(Body::TooLarge(bytes)) => format!(
                "{}...<truncated>",
                preview(&String::from_utf8_lossy(&bytes), ERROR_BODY_PREVIEW_CHARS)
            ),
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

    // BUG #6 fix: Stop reading at the size limit (a body without
    // Content-Length was read whole before its size was checked)
    let bytes = match read_body_limited(response, MAX_RESPONSE_SIZE)
        .await
        .context("Failed to read response body")?
    {
        Body::Complete(bytes) => bytes,
        Body::TooLarge(_) => {
            return Ok(Some(make_error_response(
                request.id.clone(),
                -32603,
                &format!("Response body too large (max: {} bytes)", MAX_RESPONSE_SIZE),
            )));
        }
    };

    // Empty body = notification acknowledged
    if bytes.is_empty() {
        return Ok(no_answer(request));
    }

    // Parse response
    let resp: JsonRpcResponse =
        serde_json::from_slice(&bytes).context("Failed to parse JSON-RPC response")?;
    Ok(Some(resp))
}

/// The server sent no response: right for a notification; a request with an
/// id still gets one, else the client waits for it forever
fn no_answer(request: &JsonRpcRequest) -> Option<JsonRpcResponse> {
    (!is_notification(request)).then(|| {
        make_error_response(
            request.id.clone(),
            -32603,
            "Server sent no response to the request",
        )
    })
}

/// A response body read up to a size limit
#[derive(Debug, PartialEq)]
enum Body {
    /// The whole body, at most the limit
    Complete(Vec<u8>),
    /// The body exceeded the limit: the bytes read before stopping
    TooLarge(Vec<u8>),
}

/// Read a response body, stopping once it exceeds `limit` bytes
async fn read_body_limited(mut response: reqwest::Response, limit: usize) -> Result<Body> {
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        body.extend_from_slice(&chunk);
        if body.len() > limit {
            return Ok(Body::TooLarge(body));
        }
    }
    Ok(Body::Complete(body))
}

/// At most the first `max_chars` characters of `text` (never splits one:
/// slicing at a byte index panicked inside a multi-byte character)
fn preview(text: &str, max_chars: usize) -> &str {
    match text.char_indices().nth(max_chars) {
        Some((end, _)) => &text[..end],
        None => text,
    }
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

    if let Some(json) = answer_line(client, args, line).await {
        // BUG #4 fix: Check write_response return value
        if !write_response(&json) {
            return false;
        }
    }
    true
}

/// The stdout line answering an input line: None for notifications
async fn answer_line(client: &reqwest::Client, args: &Args, line: &str) -> Option<String> {
    match parse_input(line) {
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
                return None;
            }

            match forward_single(client, args, &request).await {
                Ok(Some(resp)) => serialize_response(&resp),
                Ok(None) => None, // notification response
                Err(e) => {
                    tracing::error!("Request failed: {}", e);
                    serialize_response(&make_error_response(request.id, -32603, &e.to_string()))
                }
            }
        }
        Ok(JsonRpcInput::Batch {
            requests,
            mut errors,
        }) => {
            tracing::debug!("Processing batch of {} requests", requests.len());

            let mut responses = process_batch(client, args, requests).await;
            responses.append(&mut errors);
            if responses.is_empty() {
                return None; // all were notifications
            }
            serialize_response(&responses)
        }
        Err(e) => {
            tracing::error!("Parse error: {}", e.message());
            serialize_response(&make_error_response(e.id(), e.code(), &e.message()))
        }
    }
}

/// Main loop: forward stdin lines until EOF, a broken stdout or `shutdown`.
/// The shutdown signal also ends a request still waiting on the server: it
/// was only watched while reading stdin, so SIGTERM during a hung request
/// was ignored.
async fn run<R, S>(
    client: &reqwest::Client,
    args: &Args,
    mut lines: tokio::io::Lines<R>,
    shutdown: S,
) where
    R: tokio::io::AsyncBufRead + Unpin,
    S: std::future::Future<Output = ()>,
{
    tracing::debug!("Entering main loop...");
    tokio::pin!(shutdown);

    // BUG #4 fix: Exit on stdout broken
    loop {
        // BUG #4 fix: Check stdout before each iteration
        if STDOUT_BROKEN.load(Ordering::SeqCst) {
            tracing::info!("stdout broken, exiting...");
            break;
        }

        let line = tokio::select! {
            line_result = lines.next_line() => line_result,
            _ = &mut shutdown => {
                tracing::info!("Shutting down gracefully...");
                break;
            }
        };
        match line {
            Ok(Some(line)) if !line.trim().is_empty() => {
                tokio::select! {
                    // BUG #4 fix: Check process_line return value
                    keep_going = process_line(client, args, &line) => {
                        if !keep_going {
                            tracing::info!("stdout broken during processing, exiting...");
                            break;
                        }
                    }
                    _ = &mut shutdown => {
                        tracing::info!("Shutting down gracefully (request in flight)...");
                        break;
                    }
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
    if args.admin_key.is_some() {
        tracing::info!("Admin key: configured (injected into admin_* tool calls)");
    }

    // Build HTTP client with connection pooling
    let client = build_client(&args)?;

    // Health check with retry
    wait_for_server(&client, &args.server, args.health_retries).await?;

    // Setup stdin reader
    let stdin = tokio::io::stdin();
    let lines = BufReader::new(stdin).lines();

    run(&client, &args, lines, shutdown_signal()).await;

    tracing::info!("Bridge stopped");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    /// What the mock server answers to each POST
    #[derive(Clone, Copy)]
    enum Reply {
        /// Echo the request back as the result
        Echo,
        /// HTTP 500 with a body of `n` 'é' characters, no Content-Length
        HugeError(usize),
        /// HTTP 200 streaming `n` bytes, no Content-Length
        HugeBody(usize),
        /// Never answer
        Hang,
        /// HTTP 204, no body (what the server answers a notification)
        NoContent,
        /// HTTP 200 with an empty body
        Empty,
    }

    /// Serve `reply` on a local port; returns the /mcp URL and the request
    /// bodies received
    async fn mock_server(reply: Reply) -> (String, Arc<Mutex<Vec<serde_json::Value>>>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/mcp", listener.local_addr().unwrap());
        let received = Arc::new(Mutex::new(Vec::new()));
        let seen = received.clone();
        tokio::spawn(async move {
            loop {
                let (mut socket, _) = listener.accept().await.unwrap();
                let seen = seen.clone();
                tokio::spawn(async move {
                    let mut data = Vec::new();
                    let mut buf = [0u8; 4096];
                    let (head_end, length) = loop {
                        let n = socket.read(&mut buf).await.unwrap();
                        if n == 0 {
                            return;
                        }
                        data.extend_from_slice(&buf[..n]);
                        if let Some(pos) = data.windows(4).position(|w| w == b"\r\n\r\n") {
                            let head = String::from_utf8_lossy(&data[..pos]).to_lowercase();
                            let length = head
                                .lines()
                                .find_map(|l| l.strip_prefix("content-length:"))
                                .map_or(0, |v| v.trim().parse::<usize>().unwrap());
                            break (pos + 4, length);
                        }
                    };
                    while data.len() < head_end + length {
                        let n = socket.read(&mut buf).await.unwrap();
                        data.extend_from_slice(&buf[..n]);
                    }
                    let request: serde_json::Value =
                        serde_json::from_slice(&data[head_end..]).unwrap();
                    seen.lock().unwrap().push(request.clone());
                    let close = "Connection: close\r\n";
                    let _ = match reply {
                        Reply::Echo => {
                            let body = serde_json::json!({
                                "jsonrpc": "2.0", "id": request["id"], "result": {"echo": request}
                            })
                            .to_string();
                            socket
                                .write_all(
                                    format!(
                                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n{close}\r\n{body}",
                                        body.len()
                                    )
                                    .as_bytes(),
                                )
                                .await
                        }
                        Reply::HugeError(n) => {
                            let head = format!("HTTP/1.1 500 Internal Server Error\r\n{close}\r\n");
                            let body = format!("a{}", "\u{e9}".repeat(n));
                            socket.write_all(format!("{head}{body}").as_bytes()).await
                        }
                        Reply::HugeBody(n) => {
                            let head = format!("HTTP/1.1 200 OK\r\n{close}\r\n");
                            socket.write_all(head.as_bytes()).await.unwrap();
                            let chunk = vec![b' '; 64 * 1024];
                            let mut sent = 0;
                            while sent < n {
                                if socket.write_all(&chunk).await.is_err() {
                                    break;
                                }
                                sent += chunk.len();
                            }
                            Ok(())
                        }
                        Reply::Hang => {
                            std::future::pending::<()>().await;
                            Ok(())
                        }
                        Reply::NoContent => {
                            socket
                                .write_all(
                                    format!("HTTP/1.1 204 No Content\r\n{close}\r\n").as_bytes(),
                                )
                                .await
                        }
                        Reply::Empty => {
                            socket
                                .write_all(
                                    format!("HTTP/1.1 200 OK\r\nContent-Length: 0\r\n{close}\r\n")
                                        .as_bytes(),
                                )
                                .await
                        }
                    };
                });
            }
        });
        (url, received)
    }

    use std::sync::{Arc, Mutex};

    fn args(server: &str) -> Args {
        Args::parse_from(["ironbase-bridge", "--server", server])
    }

    fn request(json: &str) -> JsonRpcRequest {
        match parse_input(json).unwrap() {
            JsonRpcInput::Single(request) => request,
            other => panic!("not a single request: {other:?}"),
        }
    }

    /// An oversized error body is cut at a character boundary: slicing at
    /// byte 1000 panicked inside a two-byte 'é'
    #[tokio::test]
    async fn oversized_error_body_with_multibyte_text_does_not_panic() {
        let (url, _) = mock_server(Reply::HugeError(MAX_RESPONSE_SIZE)).await;
        let args = args(&url);
        let client = build_client(&args).unwrap();
        let resp = forward_single(
            &client,
            &args,
            &request(r#"{"jsonrpc":"2.0","id":1,"method":"x"}"#),
        )
        .await
        .unwrap()
        .unwrap();
        let message = resp.error.unwrap().message;
        assert!(message.starts_with("HTTP 500"), "{message}");
        assert!(
            message.ends_with("...<truncated>"),
            "{}",
            &message[message.len() - 40..]
        );
        assert_eq!(preview("aé", 1), "a");
        assert_eq!(preview("aé", 5), "aé");
    }

    /// A body without Content-Length stops being read past the limit
    #[tokio::test]
    async fn body_without_content_length_is_read_only_to_the_limit() {
        let (url, _) = mock_server(Reply::HugeBody(64 * 1024 * 1024)).await;
        let client = reqwest::Client::new();
        let response = client.post(&url).body("{}").send().await.unwrap();
        assert!(response.content_length().is_none());
        match read_body_limited(response, 1024 * 1024).await.unwrap() {
            Body::TooLarge(bytes) => assert!(bytes.len() < 2 * 1024 * 1024, "{}", bytes.len()),
            Body::Complete(bytes) => panic!("read all {} bytes", bytes.len()),
        }
    }

    /// A server that never answers fails the request after the timeout
    #[tokio::test]
    async fn hung_server_times_out() {
        let (url, _) = mock_server(Reply::Hang).await;
        let args = Args::parse_from(["ironbase-bridge", "--server", &url, "--timeout-secs", "1"]);
        let client = build_client(&args).unwrap();
        let started = std::time::Instant::now();
        let result = forward_single(
            &client,
            &args,
            &request(r#"{"jsonrpc":"2.0","id":1,"method":"x"}"#),
        )
        .await;
        assert!(result.is_err());
        assert!(started.elapsed() < Duration::from_secs(10));
    }

    /// The shutdown signal ends the loop while a request waits on a server
    /// that never answers (it was only watched while reading stdin)
    #[tokio::test]
    async fn shutdown_ends_a_request_in_flight() {
        let (url, received) = mock_server(Reply::Hang).await;
        let args = args(&url);
        let client = build_client(&args).unwrap();
        let input: &[u8] = b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"x\"}\n";
        let lines = BufReader::new(input).lines();
        let shutdown = {
            let received = received.clone();
            async move {
                while received.lock().unwrap().is_empty() {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            }
        };
        tokio::time::timeout(
            Duration::from_secs(10),
            run(&client, &args, lines, shutdown),
        )
        .await
        .expect("the loop ended on the shutdown signal");
        assert_eq!(
            received.lock().unwrap().len(),
            1,
            "the request was in flight"
        );
    }

    /// The admin key goes only into admin_* tool calls
    #[tokio::test]
    async fn admin_key_is_injected_only_into_admin_tools() {
        let (url, received) = mock_server(Reply::Echo).await;
        let args = Args::parse_from(["ironbase-bridge", "--server", &url, "--admin-key", "secret"]);
        let client = build_client(&args).unwrap();
        for (id, tool) in [
            (1, "admin_apikey_list"),
            (2, "find"),
            (3, "rag_collection_stats"),
        ] {
            let call = format!(
                r#"{{"jsonrpc":"2.0","id":{id},"method":"tools/call","params":{{"name":"{tool}","arguments":{{}}}}}}"#
            );
            forward_single(&client, &args, &request(&call))
                .await
                .unwrap();
        }
        forward_single(
            &client,
            &args,
            &request(r#"{"jsonrpc":"2.0","id":4,"method":"tools/list"}"#),
        )
        .await
        .unwrap();
        let received = received.lock().unwrap();
        let keys: Vec<_> = received
            .iter()
            .map(|r| {
                r["params"]["arguments"]["admin_key"]
                    .as_str()
                    .map(str::to_string)
            })
            .collect();
        assert_eq!(keys, vec![Some("secret".to_string()), None, None, None]);
        // An explicit key in the arguments wins
        let mut explicit = request(
            r#"{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"admin_x","arguments":{"admin_key":"mine"}}}"#,
        );
        explicit = inject_admin_key(&explicit, "secret");
        assert_eq!(explicit.params["arguments"]["admin_key"], "mine");
    }

    /// Errors without a known id carry `"id": null`; absent params stay
    /// absent when forwarded
    #[tokio::test]
    async fn json_rpc_ids_and_params() {
        let parse_error = parse_input("not json").unwrap_err();
        let resp =
            make_error_response(parse_error.id(), parse_error.code(), &parse_error.message());
        let json: serde_json::Value =
            serde_json::from_str(&serialize_response(&resp).unwrap()).unwrap();
        assert!(json.as_object().unwrap().contains_key("id"), "{json}");
        assert!(json["id"].is_null());

        let (url, received) = mock_server(Reply::Echo).await;
        let args = args(&url);
        let client = build_client(&args).unwrap();
        forward_single(
            &client,
            &args,
            &request(r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#),
        )
        .await
        .unwrap();
        let forwarded = received.lock().unwrap()[0].clone();
        assert!(forwarded.get("params").is_none(), "{forwarded}");
    }

    /// Deterministic generator (xorshift): reproducible random input lines
    struct Gen(u64);

    impl Gen {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }
        fn below(&mut self, n: u64) -> u64 {
            self.next() % n
        }
    }

    /// One request object: with an id (number or string), a notification,
    /// a null id, or a wrong `jsonrpc` version
    fn random_request(gen: &mut Gen) -> serde_json::Value {
        let mut request = serde_json::json!({"jsonrpc": "2.0", "method": "tools/call", "params": {"n": gen.below(9)}});
        match gen.below(6) {
            0 | 1 => request["id"] = serde_json::json!(gen.below(1000)),
            2 => request["id"] = serde_json::json!(format!("s{}", gen.below(1000))),
            3 => {} // notification
            4 => request["id"] = serde_json::Value::Null,
            _ => {
                request["id"] = serde_json::json!(gen.below(1000));
                request["jsonrpc"] = serde_json::json!("1.0");
            }
        }
        request
    }

    /// A line: a request, a batch of 0..4 requests, or not a request
    fn random_line(gen: &mut Gen) -> String {
        match gen.below(8) {
            0..=3 => random_request(gen).to_string(),
            4 | 5 => {
                let n = gen.below(5);
                serde_json::Value::Array((0..n).map(|_| random_request(gen)).collect()).to_string()
            }
            6 => "{not json".to_string(),
            _ => "[1, \"x\"]".to_string(),
        }
    }

    /// The ids a line's answer must carry, in some order (`null` for an
    /// invalid request or a parse error), or None when no answer is due
    fn expected_ids(line: &str) -> Option<Vec<serde_json::Value>> {
        let answer = |r: &serde_json::Value| -> Option<serde_json::Value> {
            if !r.is_object() {
                return Some(serde_json::Value::Null);
            }
            match r.get("id") {
                None => None, // notification
                Some(serde_json::Value::Null) => Some(serde_json::Value::Null),
                Some(id) => Some(id.clone()),
            }
        };
        match serde_json::from_str::<serde_json::Value>(line) {
            Err(_) => Some(vec![serde_json::Value::Null]),
            Ok(serde_json::Value::Array(items)) if items.is_empty() => {
                Some(vec![serde_json::Value::Null])
            }
            Ok(serde_json::Value::Array(items)) => {
                let ids: Vec<_> = items.iter().filter_map(answer).collect();
                (!ids.is_empty()).then_some(ids)
            }
            Ok(value) => answer(&value).map(|id| vec![id]),
        }
    }

    /// P8: every request with an id gets one answer carrying that id (also
    /// when the server sends none, a 204 or an empty body), a notification
    /// none, an invalid request an error; for single requests and batches
    #[tokio::test]
    async fn every_request_gets_one_answer_with_its_id() {
        let cases: usize = std::env::var("PROPTEST_CASES")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(300);
        let mut gen = Gen(0x9e37_79b9_7f4a_7c15);
        for reply in [Reply::Echo, Reply::NoContent, Reply::Empty] {
            let (url, _) = mock_server(reply).await;
            let args = args(&url);
            let client = build_client(&args).unwrap();
            for case in 0..cases {
                let line = random_line(&mut gen);
                let output = answer_line(&client, &args, &line).await;
                let expected = expected_ids(&line);
                let got: Option<Vec<serde_json::Value>> = output.map(|text| {
                    match serde_json::from_str::<serde_json::Value>(&text).unwrap() {
                        serde_json::Value::Array(items) => {
                            items.iter().map(|r| r["id"].clone()).collect()
                        }
                        single => vec![single["id"].clone()],
                    }
                });
                let sorted = |ids: Option<Vec<serde_json::Value>>| {
                    ids.map(|mut ids| {
                        ids.sort_by_key(|id| id.to_string());
                        ids
                    })
                };
                assert_eq!(sorted(got), sorted(expected), "case {case}, line {line}");
            }
        }
    }
}
