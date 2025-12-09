// MCP IronBase Server - Main entry point
//
// A lightweight MCP server that wraps IronBase document database.
// Supports both stdio (for Claude Desktop) and HTTP modes.
// Can be installed as a system service on Windows (Service), Linux (systemd), and macOS (launchd).
//
// Usage:
//   mcp-ironbase-server                  # HTTP server mode (default)
//   mcp-ironbase-server --stdio          # Claude Desktop mode (stdin/stdout)
//   mcp-ironbase-server install          # Install as system service
//   mcp-ironbase-server uninstall        # Uninstall system service
//   mcp-ironbase-server start            # Start the service
//   mcp-ironbase-server stop             # Stop the service
//   mcp-ironbase-server status           # Check service status

use serde::{Deserialize, Serialize};
use std::io::{BufRead, Write};
use std::path::PathBuf;
use std::sync::Arc;

use mcp_docjl::{
    dispatch_tool, get_prompt_content, get_prompts_list, get_tools_list, http_server, service,
    IronBaseAdapter, VERSION,
};

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().collect();

    // Handle service commands
    if args.len() > 1 {
        match args[1].as_str() {
            "install" => match service::install() {
                Ok(()) => std::process::exit(0),
                Err(e) => {
                    eprintln!("Error installing service: {}", e);
                    std::process::exit(1);
                }
            },
            "uninstall" => match service::uninstall() {
                Ok(()) => std::process::exit(0),
                Err(e) => {
                    eprintln!("Error uninstalling service: {}", e);
                    std::process::exit(1);
                }
            },
            "start" => match service::start() {
                Ok(()) => std::process::exit(0),
                Err(e) => {
                    eprintln!("Error starting service: {}", e);
                    std::process::exit(1);
                }
            },
            "stop" => match service::stop() {
                Ok(()) => std::process::exit(0),
                Err(e) => {
                    eprintln!("Error stopping service: {}", e);
                    std::process::exit(1);
                }
            },
            "status" => match service::status() {
                Ok(status) => {
                    println!("Service status: {}", status);
                    std::process::exit(0);
                }
                Err(e) => {
                    eprintln!("Error checking status: {}", e);
                    std::process::exit(1);
                }
            },
            "--stdio" => {
                run_stdio_server();
                return;
            }
            "--service" => {
                // Windows Service mode - called by SCM
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
            "--help" | "-h" => {
                print_help();
                std::process::exit(0);
            }
            "--version" | "-V" => {
                println!("mcp-ironbase-server v{}", VERSION);
                std::process::exit(0);
            }
            arg => {
                eprintln!("Unknown command: {}", arg);
                print_help();
                std::process::exit(1);
            }
        }
    }

    // Default: run HTTP server
    http_server::run_http_server().await;
}

fn print_help() {
    println!("IronBase MCP Server v{}", VERSION);
    println!();
    println!("USAGE:");
    println!("    mcp-ironbase-server [COMMAND]");
    println!();
    println!("COMMANDS:");
    println!("    (none)      Start HTTP server (default)");
    println!("    --stdio     Start in stdio mode (for Claude Desktop)");
    println!("    install     Install as system service");
    println!("    uninstall   Uninstall system service");
    println!("    start       Start the service");
    println!("    stop        Stop the service");
    println!("    status      Check service status");
    println!("    --help      Show this help message");
    println!("    --version   Show version");
    println!();
    println!("ENVIRONMENT:");
    println!("    IRONBASE_PATH       Database file path");
    println!("                        Default (Windows): %LOCALAPPDATA%\\IronBase\\data\\ironbase_data.mlite");
    println!("                        Default (Linux):   /var/lib/ironbase/ironbase_data.mlite");
    println!("                        Default (macOS):   /usr/local/var/ironbase/ironbase_data.mlite");
    println!("    MCP_CONFIG          Config file path (default: config.toml)");
    println!("    IRONBASE_ADMIN_KEY  Admin key for protected operations (admin_* tools)");
    println!("                        If not set, admin operations are disabled");
}

// ============================================================
// PLATFORM-SPECIFIC DEFAULT DATABASE PATH
// ============================================================

/// Get the default database path for the current platform.
/// - Windows: %LOCALAPPDATA%\IronBase\data\ironbase_data.mlite
/// - Linux: /var/lib/ironbase/ironbase_data.mlite
/// - macOS: /usr/local/var/ironbase/ironbase_data.mlite
fn get_default_db_path() -> String {
    #[cfg(target_os = "windows")]
    {
        if let Ok(local_app_data) = std::env::var("LOCALAPPDATA") {
            let mut path = PathBuf::from(local_app_data);
            path.push("IronBase");
            path.push("data");
            // Create directory if it doesn't exist
            let _ = std::fs::create_dir_all(&path);
            path.push("ironbase_data.mlite");
            return path.to_string_lossy().to_string();
        }
    }

    #[cfg(target_os = "macos")]
    {
        let path = PathBuf::from("/usr/local/var/ironbase/ironbase_data.mlite");
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        return path.to_string_lossy().to_string();
    }

    #[cfg(target_os = "linux")]
    {
        let path = PathBuf::from("/var/lib/ironbase/ironbase_data.mlite");
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
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

fn run_stdio_server() {
    // Stderr for logging (stdout is for MCP protocol)
    eprintln!("MCP IronBase Server v{} (stdio mode)", VERSION);

    // Get database path from env or use platform-specific default
    let db_path = std::env::var("IRONBASE_PATH").unwrap_or_else(|_| get_default_db_path());

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
                let error_response =
                    create_error_response(-32700, &format!("Parse error: {}", e), None);
                let _ = writeln!(
                    stdout,
                    "{}",
                    serde_json::to_string(&error_response).unwrap()
                );
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

        "initialized" | "notifications/initialized" => {
            // This is a notification - NO RESPONSE per JSON-RPC spec
            eprintln!("Received initialized notification (no response sent)");
            None
        }

        "ping" => {
            // Keep-alive ping - return empty result
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
    resources: serde_json::Value,
    logging: serde_json::Value,
}

#[derive(Debug, Serialize)]
struct ServerInfo {
    name: String,
    version: String,
}
