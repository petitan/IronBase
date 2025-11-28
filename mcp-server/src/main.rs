// MCP IronBase Server - Main entry point
//
// A lightweight MCP server that wraps IronBase document database.
// Supports both stdio (for Claude Desktop) and HTTP modes.
//
// Usage:
//   mcp-ironbase-server --stdio          # Claude Desktop mode (stdin/stdout)
//   mcp-ironbase-server                  # HTTP server mode (default)

use serde::{Deserialize, Serialize};
use std::io::{BufRead, Write};
use std::path::PathBuf;
use std::sync::Arc;

use mcp_docjl::{
    dispatch_tool, get_prompt_content, get_prompts_list, get_tools_list, IronBaseAdapter, VERSION,
};

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().collect();

    // Check for --stdio flag
    if args.iter().any(|a| a == "--stdio") {
        run_stdio_server();
    } else {
        run_http_server().await;
    }
}

// ============================================================
// STDIO MODE (for Claude Desktop)
// ============================================================

fn run_stdio_server() {
    // Stderr for logging (stdout is for MCP protocol)
    eprintln!("MCP IronBase Server v{} (stdio mode)", VERSION);

    // Get database path from env or use default
    let db_path = std::env::var("IRONBASE_PATH")
        .unwrap_or_else(|_| "ironbase_data.mlite".to_string());

    eprintln!("Database path: {}", db_path);

    // Initialize adapter
    let adapter = match IronBaseAdapter::new(&db_path) {
        Ok(a) => Arc::new(a),
        Err(e) => {
            eprintln!("Failed to create adapter: {}", e);
            std::process::exit(1);
        }
    };

    eprintln!("Ready for requests...");

    // Read from stdin line by line
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();

    for line in stdin.lock().lines() {
        let line = match line {
            Ok(l) => l,
            Err(e) => {
                eprintln!("Read error: {}", e);
                continue;
            }
        };

        // Skip empty lines
        if line.trim().is_empty() {
            continue;
        }

        // Parse request
        let request: McpRequest = match serde_json::from_str(&line) {
            Ok(r) => r,
            Err(e) => {
                let error_response = create_error_response(
                    -32700,
                    &format!("Parse error: {}", e),
                    None,
                );
                let _ = writeln!(stdout, "{}", serde_json::to_string(&error_response).unwrap());
                let _ = stdout.flush();
                continue;
            }
        };

        // Handle request - only respond if it's a request (has id), not a notification
        if let Some(response) = handle_request(&request, &adapter) {
            // Write response only for requests, not notifications
            if let Err(e) = writeln!(stdout, "{}", serde_json::to_string(&response).unwrap()) {
                eprintln!("Write error: {}", e);
            }
            let _ = stdout.flush();
        }
        // Notifications (no id) get no response - this is correct per JSON-RPC spec
    }
}

fn handle_request(request: &McpRequest, adapter: &Arc<IronBaseAdapter>) -> Option<McpResponse> {
    // Check if this is a notification (no id) - notifications get no response per JSON-RPC spec
    let is_notification = request.id.is_none()
        || matches!(&request.id, Some(v) if v.is_null());

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

        "initialized" | "notifications/initialized" => {
            // This is a notification - NO RESPONSE per JSON-RPC spec
            eprintln!("Received initialized notification (no response sent)");
            None
        }

        "ping" => {
            // Keep-alive ping - return empty result
            Some(create_success_response(serde_json::json!({}), request.id.clone()))
        }

        "notifications/cancelled" => {
            // This is a notification - NO RESPONSE per JSON-RPC spec
            eprintln!("Received cancelled notification (no response sent)");
            None
        }

        "tools/list" => Some(create_success_response(get_tools_list(), request.id.clone())),

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

        "prompts/list" => Some(create_success_response(get_prompts_list(), request.id.clone())),

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

        // Unknown method - but if it's a notification, don't respond
        _ if is_notification => {
            eprintln!("Unknown notification: {} (no response sent)", request.method);
            None
        }

        _ => Some(create_error_response(
            -32601,
            &format!("Method not found: {}", request.method),
            request.id.clone(),
        )),
    }
}

/// Create a JSON-RPC 2.0 success response
/// ALWAYS includes jsonrpc: "2.0" and id field per spec
fn create_success_response(result: serde_json::Value, id: Option<serde_json::Value>) -> McpResponse {
    McpResponse::Success {
        jsonrpc: "2.0".to_string(),
        id: id.unwrap_or(serde_json::Value::Null),
        result,
    }
}

/// Create a JSON-RPC 2.0 error response
/// ALWAYS includes jsonrpc: "2.0" and id field per spec
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

// ============================================================
// HTTP MODE (for testing/other clients)
// ============================================================

async fn run_http_server() {
    use axum::{
        extract::{Json, State},
        http::StatusCode,
        response::{IntoResponse, Response},
        routing::{get, post},
        Router,
    };
    use tracing::info;

    // Initialize tracing
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    info!("Starting MCP IronBase Server v{} (HTTP mode)", VERSION);

    // Load configuration
    let config = load_config().expect("Failed to load configuration");
    let host = config.host.clone();
    let port = config.port;

    // Initialize IronBase adapter
    let adapter = Arc::new(
        IronBaseAdapter::new(&config.database_path).expect("Failed to create IronBase adapter"),
    );

    let app_state = Arc::new(HttpAppState { adapter });

    let app = Router::new()
        .route("/mcp", post(http_handle_mcp_request))
        .route("/health", get(health_check))
        .with_state(app_state);

    let addr: std::net::SocketAddr = format!("{}:{}", host, port)
        .parse()
        .expect("Invalid address");

    info!("Server listening on {}", addr);

    axum::Server::bind(&addr)
        .serve(app.into_make_service())
        .await
        .expect("Server error");

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
}

struct HttpAppState {
    adapter: Arc<IronBaseAdapter>,
}

#[derive(Debug, Clone, Deserialize)]
struct Config {
    host: String,
    port: u16,
    database_path: PathBuf,
}

fn load_config() -> Result<Config, Box<dyn std::error::Error>> {
    let config_path = std::env::var("MCP_CONFIG").unwrap_or_else(|_| "config.toml".to_string());

    if std::path::Path::new(&config_path).exists() {
        let content = std::fs::read_to_string(&config_path)?;
        let config: Config =
            toml::from_str(&content).map_err(|e| format!("Failed to parse config: {}", e))?;
        Ok(config)
    } else {
        Ok(Config {
            host: "0.0.0.0".to_string(),
            port: 8080,
            database_path: PathBuf::from("ironbase_data.mlite"),
        })
    }
}

// ============================================================
// Shared Types
// ============================================================

#[derive(Debug, Deserialize)]
struct McpRequest {
    #[serde(default)]
    #[allow(dead_code)] // Required for JSON-RPC 2.0 deserialization
    jsonrpc: Option<String>,
    #[serde(default)]
    id: Option<serde_json::Value>,
    method: String,
    #[serde(default)]
    params: serde_json::Value,
}

#[derive(Debug, Deserialize)]
struct ToolsCallParams {
    name: String,
    #[serde(default)]
    arguments: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct PromptsGetParams {
    name: String,
    #[serde(default)]
    arguments: Option<serde_json::Value>,
}

/// JSON-RPC 2.0 Response
/// CRITICAL: jsonrpc and id fields are REQUIRED per spec - never skip them
#[derive(Debug, Serialize)]
#[serde(untagged)]
enum McpResponse {
    Success {
        jsonrpc: String,           // ALWAYS "2.0" - required by JSON-RPC 2.0 spec
        id: serde_json::Value,     // ALWAYS present (null if unknown) - required for requests
        result: serde_json::Value,
    },
    Error {
        jsonrpc: String,           // ALWAYS "2.0" - required by JSON-RPC 2.0 spec
        id: serde_json::Value,     // ALWAYS present (null if unknown) - required for requests
        error: McpErrorResponse,
    },
}

#[derive(Debug, Serialize)]
struct McpErrorResponse {
    code: i32,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<serde_json::Value>,
}

#[derive(Debug, Serialize)]
struct InitializeResult {
    #[serde(rename = "protocolVersion")]
    protocol_version: String,
    capabilities: Capabilities,
    #[serde(rename = "serverInfo")]
    server_info: ServerInfo,
}

#[derive(Debug, Serialize)]
struct Capabilities {
    tools: serde_json::Value,
    prompts: serde_json::Value,
    resources: serde_json::Value,
    logging: serde_json::Value,
}

#[derive(Debug, Serialize)]
struct ServerInfo {
    name: String,
    version: String,
}
