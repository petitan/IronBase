//! MCP response creation utilities

use crate::transport::McpResponse;

/// Create a success MCP response
pub(crate) fn create_success_response(
    result: serde_json::Value,
    id: Option<serde_json::Value>,
) -> McpResponse {
    McpResponse::success(result, id)
}

/// Create an error MCP response
pub(crate) fn create_error_response(
    code: i32,
    message: &str,
    id: Option<serde_json::Value>,
) -> McpResponse {
    McpResponse::error(code, message, id)
}
