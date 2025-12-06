//! HTTP Server module for IronBase MCP Server
//!
//! Provides HTTP server functionality that can be started with custom shutdown signals.

use crate::{shutdown, IronBaseAdapter, VERSION};
use std::path::PathBuf;
use std::sync::Arc;

/// Default max body size: 1 GB
const DEFAULT_MAX_BODY_SIZE: usize = 1024 * 1024 * 1024;

/// Configuration for HTTP server
#[derive(Debug, Clone)]
pub struct Config {
    pub host: String,
    pub port: u16,
    pub database_path: PathBuf,
    pub max_body_size: usize,
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

/// Load configuration from environment or config file
pub fn load_config() -> Result<Config, Box<dyn std::error::Error>> {
    let config_path = std::env::var("MCP_CONFIG").unwrap_or_else(|_| "config.toml".to_string());

    if std::path::Path::new(&config_path).exists() {
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

        Ok(Config {
            host: toml_config.server.host,
            port: toml_config.server.port,
            database_path: PathBuf::from(toml_config.database.path),
            max_body_size,
        })
    } else {
        // Check for IRONBASE_PATH env var
        let db_path =
            std::env::var("IRONBASE_PATH").unwrap_or_else(|_| "ironbase_data.mlite".to_string());
        Ok(Config {
            host: "0.0.0.0".to_string(),
            port: 8080,
            database_path: PathBuf::from(db_path),
            max_body_size: DEFAULT_MAX_BODY_SIZE,
        })
    }
}

#[derive(Debug, serde::Deserialize)]
struct TomlConfig {
    server: ServerConfig,
    database: DatabaseConfig,
}

#[derive(Debug, serde::Deserialize)]
struct ServerConfig {
    host: String,
    port: u16,
    /// Max body size in human-readable format: "1GB", "500MB", "10KB"
    #[serde(default)]
    max_body_size: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
struct DatabaseConfig {
    path: String,
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

    // Initialize tracing (ignore if already initialized)
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .try_init();

    info!("Starting MCP IronBase Server v{} (HTTP mode)", VERSION);

    // Load configuration
    let config = load_config().expect("Failed to load configuration");
    let host = config.host.clone();
    let port = config.port;
    let max_body_size = config.max_body_size;

    // Initialize IronBase adapter
    let adapter = Arc::new(
        IronBaseAdapter::new(&config.database_path).expect("Failed to create IronBase adapter"),
    );

    let app_state = Arc::new(HttpAppState {
        adapter: adapter.clone(),
    });

    // HTTP request handler
    async fn http_handle_mcp_request(
        State(state): State<Arc<HttpAppState>>,
        Json(request): Json<McpRequest>,
    ) -> Response {
        match handle_request(&request, &state.adapter) {
            Some(response) => (StatusCode::OK, Json(response)).into_response(),
            None => {
                // Notification - no response body per JSON-RPC spec
                // Return 204 No Content for HTTP
                StatusCode::NO_CONTENT.into_response()
            }
        }
    }

    async fn health_check() -> impl IntoResponse {
        (
            StatusCode::OK,
            Json(serde_json::json!({
                "status": "ok",
                "version": VERSION
            })),
        )
    }

    let app = Router::new()
        .route("/mcp", post(http_handle_mcp_request))
        .route("/health", get(health_check))
        .layer(DefaultBodyLimit::max(max_body_size))
        .with_state(app_state);

    let addr = format!("{}:{}", host, port);
    let listener = TcpListener::bind(&addr)
        .await
        .expect("Failed to bind address");

    info!(
        "Server listening on {} (max body size: {})",
        addr,
        format_size(max_body_size)
    );

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

    // Run server with graceful shutdown
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_future)
        .await
        .expect("Server error");

    // Graceful shutdown: checkpoint database
    info!("Shutting down gracefully...");
    if let Err(e) = adapter.checkpoint() {
        tracing::error!("Error checkpointing database: {}", e);
    }
    info!("Server stopped");
}

struct HttpAppState {
    adapter: Arc<IronBaseAdapter>,
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
    logging: serde_json::Value,
}

#[derive(Debug, serde::Serialize)]
struct ServerInfo {
    name: String,
    version: String,
}

fn handle_request(request: &McpRequest, adapter: &Arc<IronBaseAdapter>) -> Option<McpResponse> {
    use crate::{dispatch_tool, get_prompt_content, get_prompts_list, get_tools_list};

    let is_notification = request.id.is_none() || matches!(&request.id, Some(v) if v.is_null());

    match request.method.as_str() {
        "initialize" => Some(create_success_response(
            serde_json::to_value(InitializeResult {
                protocol_version: "2025-06-18".to_string(),
                capabilities: Capabilities {
                    tools: serde_json::json!({"listChanged": false}),
                    prompts: serde_json::json!({"listChanged": false}),
                    resources: serde_json::json!({}),
                    logging: serde_json::json!({}),
                },
                server_info: ServerInfo {
                    name: "ironbase-mcp".to_string(),
                    version: VERSION.to_string(),
                },
            })
            .unwrap(),
            request.id.clone(),
        )),

        "initialized" | "notifications/initialized" => None,

        "ping" => Some(create_success_response(
            serde_json::json!({}),
            request.id.clone(),
        )),

        "notifications/cancelled" => None,

        "tools/list" => Some(create_success_response(
            get_tools_list(),
            request.id.clone(),
        )),

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

            match dispatch_tool(&params.name, arguments, adapter) {
                Ok(result) => {
                    let response = serde_json::json!({
                        "content": [{
                            "type": "text",
                            "text": serde_json::to_string_pretty(&result).unwrap_or_else(|_| "{}".to_string())
                        }]
                    });
                    Some(create_success_response(response, request.id.clone()))
                }
                Err(e) => {
                    let response = serde_json::json!({
                        "content": [{
                            "type": "text",
                            "text": format!("Error: {}", e)
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
