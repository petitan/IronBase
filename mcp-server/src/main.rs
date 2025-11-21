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
    method: String,
    params: serde_json::Value,
}

/// MCP JSON-RPC response
#[derive(Debug, Serialize)]
#[serde(untagged)]
enum McpResponse {
    Success { result: serde_json::Value },
    Error { error: McpError },
}

#[derive(Debug, Serialize)]
struct McpError {
    code: String,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    details: Option<serde_json::Value>,
}

/// Handle MCP request
async fn handle_mcp_request(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(request): Json<McpRequest>,
) -> Response {
    // Extract API key from Authorization header
    let api_key_str = match extract_api_key(&headers) {
        Some(key) => key,
        None if state.config.require_auth => {
            return error_response(
                StatusCode::UNAUTHORIZED,
                "INVALID_API_KEY",
                "Missing or invalid Authorization header",
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
            return error_response(StatusCode::UNAUTHORIZED, "INVALID_API_KEY", &e.to_string());
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
    if let Err(e) = state.auth_manager.authorize(&api_key, &request.method) {
        state
            .audit_logger
            .log_auth(&api_key.name, false, Some(e.to_string()))
            .ok();
        return error_response(StatusCode::FORBIDDEN, "COMMAND_NOT_ALLOWED", &e.to_string());
    }

    // Check rate limit
    let is_write_command = !mcp_docjl::host::security::read_only_commands().contains(&request.method);
    if is_write_command {
        if let Err(e) = state.auth_manager.check_write_rate_limit(&api_key.key) {
            state
                .audit_logger
                .log_rate_limit(&api_key.name, &request.method)
                .ok();
            return error_response(StatusCode::TOO_MANY_REQUESTS, "RATE_LIMIT_EXCEEDED", &e.to_string());
        }
    } else if let Err(e) = state.auth_manager.check_rate_limit(&api_key.key) {
        state
            .audit_logger
            .log_rate_limit(&api_key.name, &request.method)
            .ok();
        return error_response(StatusCode::TOO_MANY_REQUESTS, "RATE_LIMIT_EXCEEDED", &e.to_string());
    }

    // Execute command
    let result = execute_command(&state, &request.method, &request.params).await;

    // Log to audit
    let command_result = match &result {
        Ok(_) => CommandResult::Success,
        Err(e) => CommandResult::Error {
            message: e.to_string(),
        },
    };

    if let Ok(audit_id) = state.audit_logger.log_command(
        &request.method,
        &api_key.name,
        request.params,
        command_result,
    ) {
        info!("Command logged: audit_id={}", audit_id);
    }

    // Return response
    match result {
        Ok(value) => success_response(value),
        Err(e) => error_response(StatusCode::INTERNAL_SERVER_ERROR, "COMMAND_FAILED", &e),
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

/// Create success response
fn success_response(result: serde_json::Value) -> Response {
    (StatusCode::OK, Json(McpResponse::Success { result })).into_response()
}

/// Create error response
fn error_response(status: StatusCode, code: &str, message: &str) -> Response {
    (
        status,
        Json(McpResponse::Error {
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
