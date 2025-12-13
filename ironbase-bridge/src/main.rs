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
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, BufReader};

// ============================================================================
// CLI Arguments
// ============================================================================

#[derive(Parser, Debug)]
#[command(name = "ironbase-bridge")]
#[command(version, about = "STDIO to HTTP/HTTPS bridge for MCP IronBase Server")]
struct Args {
    /// Server URL
    #[arg(short, long, env = "MCP_SERVER_URL", default_value = "http://localhost:8080/mcp")]
    server: String,

    /// API key for authentication
    #[arg(short = 'k', long, env = "IRONBASE_API_KEY")]
    api_key: Option<String>,

    /// Accept invalid/self-signed certificates
    #[arg(long, env = "MCP_INSECURE")]
    insecure: bool,

    /// Enable debug logging
    #[arg(short, long, env = "MCP_DEBUG")]
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
    Batch(Vec<JsonRpcRequest>),
}

// ============================================================================
// Helper Functions
// ============================================================================

/// Check if request is a notification (no id or null id)
fn is_notification(request: &JsonRpcRequest) -> bool {
    match &request.id {
        None => true,
        Some(v) if v.is_null() => true,
        _ => false,
    }
}

/// Create an error response
fn make_error_response(
    id: Option<serde_json::Value>,
    code: i32,
    message: &str,
) -> JsonRpcResponse {
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

/// Parse input as single request or batch
fn parse_input(line: &str) -> Result<JsonRpcInput> {
    let trimmed = line.trim();
    if trimmed.starts_with('[') {
        let requests: Vec<JsonRpcRequest> = serde_json::from_str(trimmed)
            .context("Failed to parse batch request")?;
        Ok(JsonRpcInput::Batch(requests))
    } else {
        let request: JsonRpcRequest = serde_json::from_str(trimmed)
            .context("Failed to parse request")?;
        Ok(JsonRpcInput::Single(request))
    }
}

/// Write JSON response to stdout with proper flushing
/// CRITICAL: This is the ONLY function that writes to stdout
fn write_response(json: &str) {
    let stdout = std::io::stdout();
    let mut handle = stdout.lock();
    writeln!(handle, "{}", json).ok();
    handle.flush().ok();
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
fn build_client(args: &Args) -> Result<reqwest::Client> {
    let mut builder = reqwest::Client::builder()
        .pool_max_idle_per_host(2)
        .pool_idle_timeout(Duration::from_secs(60))
        .timeout(Duration::from_secs(30))
        .use_rustls_tls();

    // Self-signed cert support
    if args.insecure {
        tracing::warn!("Accepting invalid/self-signed certificates (--insecure)");
        builder = builder
            .danger_accept_invalid_certs(true);
    }

    builder.build().context("Failed to build HTTP client")
}

/// Health check with retry logic
async fn wait_for_server(client: &reqwest::Client, url: &str, retries: u32) -> Result<()> {
    if retries == 0 {
        tracing::debug!("Health check disabled");
        return Ok(());
    }

    let health_url = url.trim_end_matches("/mcp").to_string() + "/health";
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
            tokio::time::sleep(Duration::from_millis(500 * attempt as u64)).await;
        }
    }

    tracing::warn!(
        "Server not reachable after {} attempts, continuing anyway...",
        retries
    );
    Ok(())
}

/// Forward a single request to the server
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

    // Check for errors
    if !response.status().is_success() {
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        return Ok(Some(make_error_response(
            request.id.clone(),
            -32603,
            &format!("HTTP {}: {}", status, text),
        )));
    }

    // Empty body = notification acknowledged
    let text = response.text().await.context("Failed to read response body")?;
    if text.is_empty() {
        return Ok(None);
    }

    // Parse response
    let resp: JsonRpcResponse = serde_json::from_str(&text)
        .context("Failed to parse JSON-RPC response")?;
    Ok(Some(resp))
}

/// Process a batch of requests
async fn process_batch(
    client: &reqwest::Client,
    args: &Args,
    requests: Vec<JsonRpcRequest>,
) -> Vec<JsonRpcResponse> {
    let mut responses = Vec::new();

    for request in requests {
        if is_notification(&request) {
            // Forward notification but don't collect response
            let _ = forward_single(client, args, &request).await;
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

#[cfg(unix)]
async fn shutdown_signal() {
    use tokio::signal::unix::{signal, SignalKind};
    let mut sigint = signal(SignalKind::interrupt()).expect("Failed to setup SIGINT handler");
    let mut sigterm = signal(SignalKind::terminate()).expect("Failed to setup SIGTERM handler");
    tokio::select! {
        _ = sigint.recv() => tracing::info!("Received SIGINT"),
        _ = sigterm.recv() => tracing::info!("Received SIGTERM"),
    }
}

#[cfg(windows)]
async fn shutdown_signal() {
    tokio::signal::ctrl_c()
        .await
        .expect("Failed to setup Ctrl+C handler");
    tracing::info!("Received Ctrl+C");
}

// ============================================================================
// Main Loop
// ============================================================================

async fn process_line(client: &reqwest::Client, args: &Args, line: &str) {
    let output = match parse_input(line) {
        Ok(JsonRpcInput::Single(request)) => {
            tracing::debug!("Processing request: {} (id: {:?})", request.method, request.id);

            if is_notification(&request) {
                // Forward notification but don't output response
                let _ = forward_single(client, args, &request).await;
                return;
            }

            match forward_single(client, args, &request).await {
                Ok(Some(resp)) => serde_json::to_string(&resp).ok(),
                Ok(None) => return, // notification response
                Err(e) => {
                    tracing::error!("Request failed: {}", e);
                    serde_json::to_string(&make_error_response(
                        request.id,
                        -32603,
                        &e.to_string(),
                    ))
                    .ok()
                }
            }
        }
        Ok(JsonRpcInput::Batch(requests)) => {
            tracing::debug!("Processing batch of {} requests", requests.len());

            let responses = process_batch(client, args, requests).await;
            if responses.is_empty() {
                return; // all were notifications
            }
            serde_json::to_string(&responses).ok()
        }
        Err(e) => {
            tracing::error!("Parse error: {}", e);
            serde_json::to_string(&make_error_response(
                None,
                -32700,
                &format!("Parse error: {}", e),
            ))
            .ok()
        }
    };

    if let Some(json) = output {
        write_response(&json);
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

    tracing::info!(
        "ironbase-bridge v{} starting",
        env!("CARGO_PKG_VERSION")
    );
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
    loop {
        tokio::select! {
            // Read next line from stdin
            line_result = lines.next_line() => {
                match line_result {
                    Ok(Some(line)) if !line.trim().is_empty() => {
                        process_line(&client, &args, &line).await;
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
