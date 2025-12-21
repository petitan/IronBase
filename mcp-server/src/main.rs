// MCP IronBase Server - Main entry point
//
// A lightweight MCP server that wraps IronBase document database.
// Supports both stdio (for Claude Desktop) and HTTP modes.
// Can be installed as a system service on Windows (Service), Linux (systemd), and macOS (launchd).

use clap::{Parser, Subcommand};
use serde::{Deserialize, Serialize};
use std::io::{BufRead, Write};
use std::path::PathBuf;
use std::sync::Arc;

use mcp_ironbase::{
    dispatch_tool, get_prompt_content, get_prompts_list, get_tools_list, http_server, service,
    IronBaseAdapter, VERSION,
};

// ============================================================
// CLI Definition
// ============================================================

#[derive(Parser)]
#[command(name = "mcp-ironbase-server")]
#[command(author, version = VERSION, about = "MCP server for IronBase document database")]
struct Cli {
    /// Config file path
    #[arg(short, long, env = "MCP_CONFIG", default_value = "config.toml")]
    config: PathBuf,

    /// Server port (overrides config)
    #[arg(short, long, env = "MCP_PORT")]
    port: Option<u16>,

    /// Server host (overrides config)
    #[arg(short = 'H', long, env = "MCP_HOST")]
    host: Option<String>,

    /// Database file path (overrides config)
    #[arg(short, long, env = "IRONBASE_PATH")]
    db: Option<PathBuf>,

    /// Admin key for protected operations
    #[arg(long, env = "IRONBASE_ADMIN_KEY")]
    admin_key: Option<String>,

    /// Run in stdio mode (for Claude Desktop)
    #[arg(long)]
    stdio: bool,

    /// Run as Windows service (internal use)
    #[arg(long, hide = true)]
    service: bool,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Install as system service
    Install,
    /// Uninstall system service
    Uninstall,
    /// Start the service
    Start,
    /// Stop the service
    Stop,
    /// Check service status
    Status,
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    // Handle subcommands first
    if let Some(cmd) = cli.command {
        match cmd {
            Commands::Install => match service::install() {
                Ok(()) => std::process::exit(0),
                Err(e) => {
                    eprintln!("Error installing service: {}", e);
                    std::process::exit(1);
                }
            },
            Commands::Uninstall => match service::uninstall() {
                Ok(()) => std::process::exit(0),
                Err(e) => {
                    eprintln!("Error uninstalling service: {}", e);
                    std::process::exit(1);
                }
            },
            Commands::Start => match service::start() {
                Ok(()) => std::process::exit(0),
                Err(e) => {
                    eprintln!("Error starting service: {}", e);
                    std::process::exit(1);
                }
            },
            Commands::Stop => match service::stop() {
                Ok(()) => std::process::exit(0),
                Err(e) => {
                    eprintln!("Error stopping service: {}", e);
                    std::process::exit(1);
                }
            },
            Commands::Status => match service::status() {
                Ok(status) => {
                    println!("Service status: {}", status);
                    std::process::exit(0);
                }
                Err(e) => {
                    eprintln!("Error checking status: {}", e);
                    std::process::exit(1);
                }
            },
        }
    }

    // Handle --service flag (Windows SCM)
    if cli.service {
        #[cfg(windows)]
        {
            if let Err(e) = service::windows::run_as_service() {
                eprintln!("Service error: {}", e);
                std::process::exit(1);
            }
            std::process::exit(0);
        }
        #[cfg(not(windows))]
        {
            eprintln!("--service flag is only available on Windows");
            std::process::exit(1);
        }
    }

    // Handle --stdio mode
    if cli.stdio {
        run_stdio_server(&cli);
        return;
    }

    // Set environment variables for HTTP server to pick up
    if let Some(port) = cli.port {
        std::env::set_var("MCP_PORT", port.to_string());
    }
    if let Some(host) = &cli.host {
        std::env::set_var("MCP_HOST", host);
    }
    if let Some(db) = &cli.db {
        std::env::set_var("IRONBASE_PATH", db.to_string_lossy().to_string());
    }
    if let Some(admin_key) = &cli.admin_key {
        std::env::set_var("IRONBASE_ADMIN_KEY", admin_key);
    }
    std::env::set_var("MCP_CONFIG", cli.config.to_string_lossy().to_string());

    // Default: run HTTP server
    http_server::run_http_server().await;
}

// ============================================================
// PLATFORM-SPECIFIC DEFAULT DATABASE PATH
// ============================================================

/// Get the default database path for the current platform.
/// - Windows: %LOCALAPPDATA%\IronBase\data\ironbase_data.mlite
/// - Linux: /var/lib/ironbase/ironbase_data.mlite
/// - macOS: /usr/local/var/ironbase/ironbase_data.mlite
///
/// BUG #13 fix: Log warnings when directory creation fails instead of silent ignore
fn get_default_db_path() -> String {
    #[cfg(target_os = "windows")]
    {
        if let Ok(local_app_data) = std::env::var("LOCALAPPDATA") {
            let mut path = PathBuf::from(local_app_data);
            path.push("IronBase");
            path.push("data");
            // Create directory if it doesn't exist
            if let Err(e) = std::fs::create_dir_all(&path) {
                eprintln!("Warning: Failed to create data directory {:?}: {}", path, e);
            }
            path.push("ironbase_data.mlite");
            return path.to_string_lossy().to_string();
        }
    }

    #[cfg(target_os = "macos")]
    {
        let path = PathBuf::from("/usr/local/var/ironbase/ironbase_data.mlite");
        if let Some(parent) = path.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                eprintln!("Warning: Failed to create data directory {:?}: {}", parent, e);
            }
        }
        return path.to_string_lossy().to_string();
    }

    #[cfg(target_os = "linux")]
    {
        let path = PathBuf::from("/var/lib/ironbase/ironbase_data.mlite");
        if let Some(parent) = path.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                eprintln!("Warning: Failed to create data directory {:?}: {}", parent, e);
            }
        }
        return path.to_string_lossy().to_string();
    }

    // Fallback for other platforms
    #[allow(unreachable_code)]
    "ironbase_data.mlite".to_string()
}

// ============================================================
// STDIO MODE (for Claude Desktop)
// ============================================================

fn run_stdio_server(cli: &Cli) {
    // Stderr for logging (stdout is for MCP protocol)
    eprintln!("MCP IronBase Server v{} (stdio mode)", VERSION);

    // Get database path: CLI > env > platform default
    let db_path = cli
        .db
        .as_ref()
        .map(|p| p.to_string_lossy().to_string())
        .or_else(|| std::env::var("IRONBASE_PATH").ok())
        .unwrap_or_else(get_default_db_path);

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

    // MCP lifecycle state: track if initialize has been called
    let mut initialized = false;

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
                let error_response =
                    create_error_response(-32700, &format!("Parse error: {}", e), None);
                // BUG #14 fix: Handle JSON serialization errors gracefully
                let response_str = serde_json::to_string(&error_response)
                    .unwrap_or_else(|_| r#"{"jsonrpc":"2.0","error":{"code":-32700,"message":"Parse error"}}"#.to_string());
                let _ = writeln!(stdout, "{}", response_str);
                let _ = stdout.flush();
                continue;
            }
        };

        // Handle request with lifecycle enforcement
        if let Some(response) = handle_request(&request, &adapter, &mut initialized) {
            // BUG #14 fix: Handle JSON serialization errors gracefully
            let response_str = serde_json::to_string(&response)
                .unwrap_or_else(|e| {
                    eprintln!("Response serialization error: {}", e);
                    r#"{"jsonrpc":"2.0","error":{"code":-32603,"message":"Internal error"}}"#.to_string()
                });
            // Write response only for requests, not notifications
            if let Err(e) = writeln!(stdout, "{}", response_str) {
                eprintln!("Write error: {}", e);
            }
            let _ = stdout.flush();
        }
        // Notifications (no id) get no response - this is correct per JSON-RPC spec
    }
}

fn handle_request(
    request: &McpRequest,
    adapter: &Arc<IronBaseAdapter>,
    initialized: &mut bool,
) -> Option<McpResponse> {
    // Check if this is a notification (no id) - notifications get no response per JSON-RPC spec
    let is_notification = request.id.is_none() || matches!(&request.id, Some(v) if v.is_null());

    // MCP lifecycle enforcement: only allow initialize and ping before initialization
    // Per spec: "The initialization phase MUST be the first interaction"
    if !*initialized
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
            *initialized = true;
            eprintln!("Server initialized");
            // BUG #14 fix: Handle serialization error gracefully
            let init_result = serde_json::to_value(InitializeResult {
                protocol_version: "2025-06-18".to_string(),
                capabilities: Capabilities {
                    tools: serde_json::json!({"listChanged": false}),
                    prompts: serde_json::json!({"listChanged": false}),
                },
                server_info: ServerInfo {
                    name: "ironbase-mcp".to_string(),
                    version: VERSION.to_string(),
                },
            })
            .unwrap_or_else(|e| {
                eprintln!("Initialize result serialization error: {}", e);
                serde_json::json!({
                    "protocolVersion": "2025-06-18",
                    "capabilities": {},
                    "serverInfo": {"name": "ironbase-mcp", "version": VERSION}
                })
            });
            Some(create_success_response(init_result, request.id.clone()))
        }

        "initialized" | "notifications/initialized" => {
            // This is a notification - NO RESPONSE per JSON-RPC spec
            eprintln!("Received initialized notification (no response sent)");
            None
        }

        "ping" => {
            // Keep-alive ping - allowed before initialization
            Some(create_success_response(
                serde_json::json!({}),
                request.id.clone(),
            ))
        }

        "notifications/cancelled" => {
            // This is a notification - NO RESPONSE per JSON-RPC spec
            eprintln!("Received cancelled notification (no response sent)");
            None
        }

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

            match dispatch_tool(&params.name, arguments, adapter, None, None) {
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

        // Unknown method - but if it's a notification, don't respond
        _ if is_notification => {
            eprintln!(
                "Unknown notification: {} (no response sent)",
                request.method
            );
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
// Shared Types (for stdio mode)
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
        jsonrpc: String,       // ALWAYS "2.0" - required by JSON-RPC 2.0 spec
        id: serde_json::Value, // ALWAYS present (null if unknown) - required for requests
        result: serde_json::Value,
    },
    Error {
        jsonrpc: String,       // ALWAYS "2.0" - required by JSON-RPC 2.0 spec
        id: serde_json::Value, // ALWAYS present (null if unknown) - required for requests
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
    // Note: resources and logging are intentionally omitted as we don't implement them
}

#[derive(Debug, Serialize)]
struct ServerInfo {
    name: String,
    version: String,
}
