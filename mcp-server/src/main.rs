// MCP DOCJL Server - Main entry point

mod commands;

use axum::{
    extract::{Json, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::post,
    Router,
};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use tracing::{info, warn};

use mcp_docjl::{ApiKey, AuditLogger, AuthManager, CommandResult, IronBaseAdapter};

#[tokio::main]
async fn main() {
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    info!("Starting MCP DOCJL Server v{}", mcp_docjl::VERSION);

    // Load configuration
    let config = load_config().expect("Failed to load configuration");

    // Save host and port before moving config
    let host = config.host.clone();
    let port = config.port;

    // Initialize auth manager
    let auth_manager = Arc::new(AuthManager::new());
    auth_manager.load_api_keys(config.api_keys.clone());

    // Initialize audit logger
    let audit_logger = Arc::new(
        AuditLogger::new(config.audit_log_path.clone()).expect("Failed to create audit logger"),
    );

    // Initialize IronBase adapter
    let adapter = Arc::new(RwLock::new(
        IronBaseAdapter::new(config.ironbase_path.clone(), "documents".to_string())
            .expect("Failed to create IronBase adapter"),
    ));

    // Create application state
    let app_state = Arc::new(AppState {
        auth_manager,
        audit_logger,
        adapter,
        config,
    });

    // Build router
    let app = Router::new()
        .route("/mcp", post(handle_mcp_request))
        .route("/health", axum::routing::get(health_check))
        .with_state(app_state);

    // Bind to address
    let addr: SocketAddr = format!("{}:{}", host, port)
        .parse()
        .expect("Invalid address");

    info!("Server listening on {}", addr);

    // Start server (Axum 0.6 API)
    axum::Server::bind(&addr)
        .serve(app.into_make_service())
        .await
        .expect("Server error");
}

/// Application state
struct AppState {
    auth_manager: Arc<AuthManager>,
    audit_logger: Arc<AuditLogger>,
    adapter: Arc<RwLock<IronBaseAdapter>>,
    config: Config,
}

/// Configuration
#[derive(Debug, Clone, Deserialize)]
struct Config {
    host: String,
    port: u16,
    api_keys: Vec<ApiKey>,
    audit_log_path: PathBuf,
    ironbase_path: PathBuf,
    require_auth: bool,
}

fn load_config() -> Result<Config, Box<dyn std::error::Error>> {
    // Try to load from config.toml, fallback to defaults
    let config_path = std::env::var("MCP_CONFIG").unwrap_or_else(|_| "config.toml".to_string());

    if std::path::Path::new(&config_path).exists() {
        let content = std::fs::read_to_string(&config_path)?;
        let config: Config = toml::from_str(&content)
            .map_err(|e| format!("Failed to parse config: {}", e))?;
        Ok(config)
    } else {
        warn!("Config file not found, using defaults");
        Ok(default_config())
    }
}

fn default_config() -> Config {
    Config {
        host: "127.0.0.1".to_string(),
        port: 8080,
        api_keys: vec![],
        audit_log_path: PathBuf::from("audit.log"),
        ironbase_path: PathBuf::from("docjl_storage.mlite"),
        require_auth: false,
    }
}

/// MCP JSON-RPC request
#[derive(Debug, Deserialize)]
struct McpRequest {
    #[serde(default)]
    jsonrpc: Option<String>,
    #[serde(default)]
    id: Option<serde_json::Value>,
    method: String,
    params: serde_json::Value,
}

/// Tools/call wrapper params (MCP protocol)
#[derive(Debug, Deserialize)]
struct ToolsCallParams {
    name: String,
    #[serde(default)]
    arguments: Option<serde_json::Value>,
}

/// MCP JSON-RPC response
#[derive(Debug, Serialize)]
#[serde(untagged)]
enum McpResponse {
    Success {
        #[serde(skip_serializing_if = "Option::is_none")]
        jsonrpc: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<serde_json::Value>,
        result: serde_json::Value,
    },
    Error {
        #[serde(skip_serializing_if = "Option::is_none")]
        jsonrpc: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<serde_json::Value>,
        error: McpError,
    },
}

#[derive(Debug, Serialize)]
struct McpError {
    code: String,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    details: Option<serde_json::Value>,
}

/// MCP initialize result
#[derive(Debug, Serialize)]
struct InitializeResult {
    #[serde(rename = "protocolVersion")]
    protocol_version: String,
    capabilities: Capabilities,
    #[serde(rename = "serverInfo")]
    server_info: ServerInfo,
}

/// MCP server capabilities
#[derive(Debug, Serialize)]
struct Capabilities {
    tools: serde_json::Value,
    resources: serde_json::Value,
    prompts: serde_json::Value,
}

/// MCP server info
#[derive(Debug, Serialize)]
struct ServerInfo {
    name: String,
    version: String,
}

/// Handle MCP request
async fn handle_mcp_request(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(request): Json<McpRequest>,
) -> Response {
    // Unwrap tools/call wrapper if present (MCP protocol support)
    let (actual_method, actual_params) = if request.method == "tools/call" {
        // Parse tools/call params
        match serde_json::from_value::<ToolsCallParams>(request.params.clone()) {
            Ok(tools_params) => (
                tools_params.name,
                tools_params.arguments.unwrap_or_else(|| serde_json::json!({})),
            ),
            Err(e) => {
                return error_response_with_id(
                    StatusCode::BAD_REQUEST,
                    "INVALID_TOOLS_CALL",
                    &format!("Invalid tools/call params: {}", e),
                    request.jsonrpc,
                    request.id,
                );
            }
        }
    } else {
        // Direct method call (backward compatibility)
        (request.method.clone(), request.params.clone())
    };

    // Handle MCP protocol methods (initialize, tools/list, resources/*, prompts/list)
    // These don't require authentication or rate limiting
    if is_mcp_protocol_method(&actual_method) {
        return handle_mcp_protocol_method(
            &actual_method,
            &actual_params,
            request.jsonrpc,
            request.id,
            &state,
        ).await;
    }

    // Extract API key from Authorization header
    let api_key_str = match extract_api_key(&headers) {
        Some(key) => key,
        None if state.config.require_auth => {
            return error_response_with_id(
                StatusCode::UNAUTHORIZED,
                "INVALID_API_KEY",
                "Missing or invalid Authorization header",
                request.jsonrpc,
                request.id,
            );
        }
        None => "anonymous", // Allow anonymous if auth not required
    };

    // Authenticate
    let api_key = match state.auth_manager.authenticate(api_key_str) {
        Ok(key) => key,
        Err(e) if state.config.require_auth => {
            state
                .audit_logger
                .log_auth("unknown", false, Some(e.to_string()))
                .ok();
            return error_response_with_id(
                StatusCode::UNAUTHORIZED,
                "INVALID_API_KEY",
                &e.to_string(),
                request.jsonrpc,
                request.id,
            );
        }
        Err(_) => ApiKey {
            key: "anonymous".to_string(),
            name: "Anonymous".to_string(),
            allowed_commands: Some(mcp_docjl::host::security::default_whitelist()),
            allowed_documents: Some(["*".to_string()].iter().cloned().collect()),
            rate_limit: None,
        },
    };

    // Authorize command
    if let Err(e) = state.auth_manager.authorize(&api_key, &actual_method) {
        state
            .audit_logger
            .log_auth(&api_key.name, false, Some(e.to_string()))
            .ok();
        return error_response_with_id(
            StatusCode::FORBIDDEN,
            "COMMAND_NOT_ALLOWED",
            &e.to_string(),
            request.jsonrpc,
            request.id,
        );
    }

    // Check rate limit
    let is_write_command = !mcp_docjl::host::security::read_only_commands().contains(&actual_method);
    if is_write_command {
        if let Err(e) = state.auth_manager.check_write_rate_limit(&api_key.key) {
            state
                .audit_logger
                .log_rate_limit(&api_key.name, &actual_method)
                .ok();
            return error_response_with_id(
                StatusCode::TOO_MANY_REQUESTS,
                "RATE_LIMIT_EXCEEDED",
                &e.to_string(),
                request.jsonrpc,
                request.id,
            );
        }
    } else if let Err(e) = state.auth_manager.check_rate_limit(&api_key.key) {
        state
            .audit_logger
            .log_rate_limit(&api_key.name, &actual_method)
            .ok();
        return error_response_with_id(
            StatusCode::TOO_MANY_REQUESTS,
            "RATE_LIMIT_EXCEEDED",
            &e.to_string(),
            request.jsonrpc,
            request.id,
        );
    }

    // Execute command
    let result = execute_command(&state, &actual_method, &actual_params).await;

    // Log to audit
    let command_result = match &result {
        Ok(_) => CommandResult::Success,
        Err(e) => CommandResult::Error {
            message: e.to_string(),
        },
    };

    if let Ok(audit_id) = state.audit_logger.log_command(
        &actual_method,
        &api_key.name,
        actual_params,
        command_result,
    ) {
        info!("Command logged: audit_id={}", audit_id);
    }

    // Return response
    match result {
        Ok(value) => success_response_with_id(value, request.jsonrpc, request.id),
        Err(e) => error_response_with_id(
            StatusCode::INTERNAL_SERVER_ERROR,
            "COMMAND_FAILED",
            &e,
            request.jsonrpc,
            request.id,
        ),
    }
}

/// Execute MCP command
async fn execute_command(
    state: &AppState,
    method: &str,
    params: &serde_json::Value,
) -> Result<serde_json::Value, String> {
    info!("Executing command: {}", method);

    // Dispatch to command handlers
    let mut adapter = state.adapter.write();
    commands::dispatch_command(
        method,
        params.clone(),
        &mut adapter,
        state.audit_logger.path(),
    )
}

/// Extract API key from Authorization header
fn extract_api_key(headers: &HeaderMap) -> Option<&str> {
    headers
        .get("Authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
}

/// Create success response with JSON-RPC fields
fn success_response_with_id(
    result: serde_json::Value,
    jsonrpc: Option<String>,
    id: Option<serde_json::Value>,
) -> Response {
    (
        StatusCode::OK,
        Json(McpResponse::Success {
            jsonrpc,
            id,
            result,
        }),
    )
        .into_response()
}

/// Create error response with JSON-RPC fields
fn error_response_with_id(
    status: StatusCode,
    code: &str,
    message: &str,
    jsonrpc: Option<String>,
    id: Option<serde_json::Value>,
) -> Response {
    (
        status,
        Json(McpResponse::Error {
            jsonrpc,
            id,
            error: McpError {
                code: code.to_string(),
                message: message.to_string(),
                details: None,
            },
        }),
    )
        .into_response()
}

/// Health check endpoint
async fn health_check() -> impl IntoResponse {
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "status": "ok",
            "version": mcp_docjl::VERSION
        })),
    )
}

/// Check if method is an MCP protocol method (not a command)
fn is_mcp_protocol_method(method: &str) -> bool {
    matches!(method, "initialize" | "tools/list" | "resources/list" | "resources/read" | "prompts/list")
}

/// Handle MCP protocol methods (initialize, tools/list, resources/list, etc.)
async fn handle_mcp_protocol_method(
    method: &str,
    params: &serde_json::Value,
    jsonrpc: Option<String>,
    id: Option<serde_json::Value>,
    state: &AppState,
) -> Response {
    info!("Handling MCP protocol method: {}", method);

    match method {
        "initialize" => {
            // Return MCP initialize response
            let result = InitializeResult {
                protocol_version: "2024-11-05".to_string(),
                capabilities: Capabilities {
                    tools: serde_json::json!({}),
                    resources: serde_json::json!({}),
                    prompts: serde_json::json!({}),
                },
                server_info: ServerInfo {
                    name: "docjl-editor".to_string(),
                    version: mcp_docjl::VERSION.to_string(),
                },
            };

            success_response_with_id(
                serde_json::to_value(result).unwrap(),
                jsonrpc,
                id,
            )
        }
        "tools/list" => {
            // Return list of available MCP tools
            let tools_list = get_tools_list();
            success_response_with_id(
                serde_json::json!({
                    "tools": tools_list
                }),
                jsonrpc,
                id,
            )
        }
        "resources/list" => {
            // List all documents as MCP resources
            // Call mcp_docjl_list_documents to get document list
            let list_params = serde_json::json!({});
            let mut adapter = state.adapter.write();

            match commands::dispatch_command(
                "mcp_docjl_list_documents",
                list_params,
                &mut adapter,
                state.audit_logger.path(),
            ) {
                Ok(result) => {
                    // Extract documents from result
                    let documents = result.get("documents")
                        .and_then(|d| d.as_array())
                        .unwrap_or(&vec![])
                        .clone();

                    // Convert documents to MCP resources
                    let resources: Vec<serde_json::Value> = documents.iter()
                        .filter_map(|doc| {
                            let doc_id = doc.get("id")?.as_str()?;
                            let title = doc.get("metadata")
                                .and_then(|m| m.get("title"))
                                .and_then(|t| t.as_str())
                                .unwrap_or(doc_id);

                            // Generate description from metadata
                            let version = doc.get("metadata")
                                .and_then(|m| m.get("version"))
                                .and_then(|v| v.as_str())
                                .unwrap_or("unknown");
                            let description = format!("DOCJL Document: {} (version {})", title, version);

                            Some(serde_json::json!({
                                "uri": format!("docjl://document/{}", doc_id),
                                "name": title,
                                "description": description,
                                "mimeType": "application/json"
                            }))
                        })
                        .collect();

                    success_response_with_id(
                        serde_json::json!({
                            "resources": resources
                        }),
                        jsonrpc,
                        id,
                    )
                }
                Err(e) => {
                    error_response_with_id(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "RESOURCE_LIST_ERROR",
                        &format!("Failed to list resources: {}", e),
                        jsonrpc,
                        id,
                    )
                }
            }
        }
        "resources/read" => {
            // Parse URI from params
            let uri = match params.get("uri").and_then(|u| u.as_str()) {
                Some(u) => u,
                None => {
                    return error_response_with_id(
                        StatusCode::BAD_REQUEST,
                        "INVALID_PARAMS",
                        "Missing 'uri' parameter",
                        jsonrpc,
                        id,
                    );
                }
            };

            // Extract doc_id from URI: docjl://document/{doc_id}
            let doc_id = if let Some(stripped) = uri.strip_prefix("docjl://document/") {
                stripped
            } else {
                return error_response_with_id(
                    StatusCode::BAD_REQUEST,
                    "INVALID_URI",
                    &format!("Invalid URI format: {}. Expected: docjl://document/{{id}}", uri),
                    jsonrpc,
                    id,
                );
            };

            // Get document
            let get_params = serde_json::json!({
                "document_id": doc_id
            });

            let mut adapter = state.adapter.write();
            match commands::dispatch_command(
                "mcp_docjl_get_document",
                get_params,
                &mut adapter,
                state.audit_logger.path(),
            ) {
                Ok(result) => {
                    // Return resource with document as JSON text
                    success_response_with_id(
                        serde_json::json!({
                            "contents": [{
                                "uri": uri,
                                "mimeType": "application/json",
                                "text": serde_json::to_string_pretty(&result).unwrap_or_else(|_| "{}".to_string())
                            }]
                        }),
                        jsonrpc,
                        id,
                    )
                }
                Err(e) => {
                    // Check if document not found
                    let error_msg = e.to_string();
                    if error_msg.contains("not found") || error_msg.contains("does not exist") {
                        error_response_with_id(
                            StatusCode::NOT_FOUND,
                            "RESOURCE_NOT_FOUND",
                            &format!("Document '{}' not found", doc_id),
                            jsonrpc,
                            id,
                        )
                    } else {
                        error_response_with_id(
                            StatusCode::INTERNAL_SERVER_ERROR,
                            "RESOURCE_READ_ERROR",
                            &format!("Failed to read resource: {}", e),
                            jsonrpc,
                            id,
                        )
                    }
                }
            }
        }
        "prompts/list" => {
            let prompts_list = get_prompts_list();
            success_response_with_id(
                serde_json::json!({
                    "prompts": prompts_list
                }),
                jsonrpc,
                id,
            )
        }
        _ => error_response_with_id(
            StatusCode::NOT_FOUND,
            "METHOD_NOT_FOUND",
            &format!("Unknown MCP method: {}", method),
            jsonrpc,
            id,
        ),
    }
}

/// Get list of available MCP tools
fn get_tools_list() -> Vec<serde_json::Value> {
    vec![
        serde_json::json!({
            "name": "mcp_docjl_create_document",
            "description": "Create a new DOCJL document",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "document": {
                        "type": "object",
                        "description": "Full document with id, metadata, and docjll blocks",
                        "properties": {
                            "id": {"type": "string", "description": "Document identifier"},
                            "metadata": {"type": "object", "description": "Document metadata (title, version, etc.)"},
                            "docjll": {"type": "array", "description": "Array of top-level blocks"}
                        },
                        "required": ["id", "metadata", "docjll"]
                    }
                },
                "required": ["document"]
            }
        }),
        serde_json::json!({
            "name": "mcp_docjl_list_documents",
            "description": "List all DOCJL documents",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "filter": {"type": "object", "description": "Optional filter"}
                }
            }
        }),
        serde_json::json!({
            "name": "mcp_docjl_get_document",
            "description": "Get full DOCJL document by ID",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "document_id": {"type": "string", "description": "Document ID"}
                },
                "required": ["document_id"]
            }
        }),
        serde_json::json!({
            "name": "mcp_docjl_list_headings",
            "description": "Get document outline/table of contents",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "document_id": {"type": "string", "description": "Document ID"}
                },
                "required": ["document_id"]
            }
        }),
        serde_json::json!({
            "name": "mcp_docjl_search_blocks",
            "description": "Search for blocks in documents",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "document_id": {"type": "string", "description": "Document ID"},
                    "query": {"type": "object", "description": "Search query"}
                },
                "required": ["document_id", "query"]
            }
        }),
        serde_json::json!({
            "name": "mcp_docjl_search_content",
            "description": "Search for text content within a document. Returns only matching blocks to solve context window problems. Case-insensitive by default.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "document_id": {"type": "string", "description": "Document ID to search in"},
                    "query": {"type": "string", "description": "Text to search for"},
                    "case_sensitive": {"type": "boolean", "description": "Whether search should be case-sensitive (default: false)"},
                    "max_results": {"type": "integer", "description": "Maximum number of matches to return (default: 100)"}
                },
                "required": ["document_id", "query"]
            }
        }),
        serde_json::json!({
            "name": "mcp_docjl_insert_block",
            "description": "Insert new content block into document. Label format: 'type:id' - supports numeric (para:1), hierarchical (sec:4.2.1), or alphanumeric (para:test, sec:demo_1) identifiers.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "document_id": {"type": "string", "description": "Document ID (as string)"},
                    "block": {
                        "type": "object",
                        "description": "Block to insert with type, label (format: 'type:id' - numeric, hierarchical, or alphanumeric), and content array",
                        "properties": {
                            "type": {"type": "string", "enum": ["paragraph", "heading"], "description": "Block type"},
                            "label": {"type": "string", "pattern": "^(para|sec|fig|tbl|eq|lst|def|thm|lem|proof|ex|note|warn|info|tip):([a-zA-Z0-9._]+)$", "description": "Label in format 'type:id' (e.g. para:1, sec:4.2.1, para:test, sec:demo_1)"},
                            "content": {"type": "array", "description": "Content array with {type, content} objects"}
                        },
                        "required": ["type", "label", "content"]
                    },
                    "position": {"type": "string", "description": "Insert position: 'start', 'end', or 'before:label' / 'after:label'"}
                },
                "required": ["document_id", "block"]
            }
        }),
        serde_json::json!({
            "name": "mcp_docjl_update_block",
            "description": "Update existing block",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "document_id": {"type": "string"},
                    "block_label": {"type": "string"},
                    "updates": {"type": "object"}
                },
                "required": ["document_id", "block_label", "updates"]
            }
        }),
        serde_json::json!({
            "name": "mcp_docjl_delete_block",
            "description": "Delete block from document",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "document_id": {"type": "string"},
                    "block_label": {"type": "string"}
                },
                "required": ["document_id", "block_label"]
            }
        }),
        serde_json::json!({
            "name": "mcp_docjl_get_section",
            "description": "Get specific section with children (Phase 3.1: Chunking Support). Helps work with large documents by retrieving only specific sections with controlled depth to fit context window.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "document_id": {"type": "string", "description": "Document ID"},
                    "section_label": {"type": "string", "description": "Section label to retrieve (e.g., 'sec:4.2.1')"},
                    "include_subsections": {"type": "boolean", "description": "Whether to include child blocks (default: true)"},
                    "max_depth": {"type": "integer", "description": "Maximum depth of children to include (default: 10)"}
                },
                "required": ["document_id", "section_label"]
            }
        }),
        serde_json::json!({
            "name": "mcp_docjl_estimate_tokens",
            "description": "Estimate token count for document or section (Phase 3.2: Chunking Support). Helps plan context usage by estimating tokens before retrieval.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "document_id": {"type": "string", "description": "Document ID"},
                    "section_label": {"type": "string", "description": "Optional: Specific section label to estimate (if omitted, estimates entire document)"}
                },
                "required": ["document_id"]
            }
        }),
    ]
}

/// Get MCP prompts list (15 prompts: 10 Balanced + 5 Calibration)
fn get_prompts_list() -> Vec<serde_json::Value> {
    vec![
        // Balanced MVP (10 prompts)
        serde_json::json!({"name": "validate-structure", "description": "Validate DOCJL document structure and label hierarchy", "arguments": [{"name": "document_id", "description": "Document ID to validate", "required": true}]}),
        serde_json::json!({"name": "validate-compliance", "description": "Check ISO 17025 compliance requirements", "arguments": [{"name": "document_id", "description": "Document ID (Quality Manual, SOP, etc.)", "required": true}, {"name": "document_type", "description": "Type: quality_manual | sop | work_instruction | audit_report", "required": true}]}),
        serde_json::json!({"name": "create-section", "description": "Create new DOCJL section with appropriate structure", "arguments": [{"name": "section_type", "description": "Type: procedure | requirement | evidence | reference", "required": true}, {"name": "topic", "description": "Section topic/title", "required": true}]}),
        serde_json::json!({"name": "summarize-document", "description": "Generate executive summary of DOCJL document", "arguments": [{"name": "document_id", "description": "Document ID to summarize", "required": true}, {"name": "max_length", "description": "Maximum summary length in words", "required": false}]}),
        serde_json::json!({"name": "suggest-improvements", "description": "Analyze document and suggest improvements", "arguments": [{"name": "document_id", "description": "Document ID to analyze", "required": true}, {"name": "focus_areas", "description": "Areas to focus: clarity | completeness | compliance | consistency", "required": false}]}),
        serde_json::json!({"name": "audit-readiness", "description": "Check document readiness for ISO 17025 audit", "arguments": [{"name": "document_id", "description": "Document ID to check", "required": true}, {"name": "audit_scope", "description": "Audit scope: full | partial | specific_clause", "required": false}]}),
        serde_json::json!({"name": "create-outline", "description": "Generate document outline based on document type", "arguments": [{"name": "document_type", "description": "Type: quality_manual | sop | work_instruction", "required": true}, {"name": "topic", "description": "Document topic", "required": true}]}),
        serde_json::json!({"name": "analyze-changes", "description": "Compare two document versions and analyze changes", "arguments": [{"name": "document_id_old", "description": "Old version document ID", "required": true}, {"name": "document_id_new", "description": "New version document ID", "required": true}]}),
        serde_json::json!({"name": "check-consistency", "description": "Check internal consistency across document sections", "arguments": [{"name": "document_id", "description": "Document ID to check", "required": true}]}),
        serde_json::json!({"name": "resolve-reference", "description": "Resolve and explain a label reference", "arguments": [{"name": "document_id", "description": "Document ID", "required": true}, {"name": "label", "description": "Label to resolve (e.g. sec:4.2.1)", "required": true}, {"name": "include_context", "description": "Include surrounding context", "required": false}]}),
        
        // Calibration-specific (5 prompts)
        serde_json::json!({"name": "calculate-measurement-uncertainty", "description": "Calculate measurement uncertainty for calibration data", "arguments": [{"name": "measurement_data", "description": "Measurement data array", "required": true}, {"name": "instrument_specs", "description": "Instrument specifications", "required": true}, {"name": "environmental_data", "description": "Environmental conditions", "required": false}]}),
        serde_json::json!({"name": "generate-calibration-hierarchy", "description": "Generate traceability hierarchy for calibration", "arguments": [{"name": "instrument_type", "description": "Type of instrument", "required": true}, {"name": "measurement_range", "description": "Measurement range", "required": true}, {"name": "criticality", "description": "Criticality: high | medium | low", "required": false}]}),
        serde_json::json!({"name": "determine-calibration-interval", "description": "Determine optimal calibration interval", "arguments": [{"name": "instrument_data", "description": "Instrument usage and history data", "required": true}, {"name": "risk_tolerance", "description": "Risk tolerance level", "required": false}]}),
        serde_json::json!({"name": "create-calibration-certificate", "description": "Generate calibration certificate from data", "arguments": [{"name": "instrument_data", "description": "Instrument information", "required": true}, {"name": "calibration_results", "description": "Calibration results", "required": true}, {"name": "uncertainty_data", "description": "Uncertainty budget data", "required": true}, {"name": "reference_standard", "description": "Reference standard details", "required": true}, {"name": "environmental_conditions", "description": "Environmental conditions during calibration", "required": false}]}),
        serde_json::json!({"name": "generate-uncertainty-budget", "description": "Create detailed measurement uncertainty budget", "arguments": [{"name": "measurement_process", "description": "Description of measurement process", "required": true}, {"name": "uncertainty_components", "description": "List of uncertainty sources", "required": true}]}),
    ]
}
