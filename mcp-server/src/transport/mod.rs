//! Transport abstraction for MCP protocol
//!
//! Provides common types and traits for different transport mechanisms (stdio, HTTP).

pub mod handler;

use serde::{Deserialize, Serialize};

// ============================================================
// JSON-RPC 2.0 Request/Response Types
// ============================================================

/// JSON-RPC 2.0 Request
#[derive(Debug, Clone, Deserialize)]
pub struct McpRequest {
    /// JSON-RPC version (always "2.0")
    #[serde(default)]
    #[allow(dead_code)]
    pub jsonrpc: Option<String>,

    /// Request ID (None for notifications)
    #[serde(default)]
    pub id: Option<serde_json::Value>,

    /// Method name
    pub method: String,

    /// Method parameters
    #[serde(default)]
    pub params: serde_json::Value,
}

impl McpRequest {
    /// Check if this is a notification (no id)
    #[inline]
    pub fn is_notification(&self) -> bool {
        self.id.is_none()
    }

    /// Check if the id is explicitly null (invalid per spec)
    #[inline]
    pub fn has_null_id(&self) -> bool {
        matches!(&self.id, Some(v) if v.is_null())
    }
}

/// JSON-RPC 2.0 Response
/// CRITICAL: jsonrpc and id fields are REQUIRED per spec - never skip them
#[derive(Debug, Clone, Serialize)]
#[serde(untagged)]
pub enum McpResponse {
    /// Successful response
    Success {
        /// JSON-RPC version (always "2.0")
        jsonrpc: String,
        /// Request ID (null if unknown)
        id: serde_json::Value,
        /// Result value
        result: serde_json::Value,
    },
    /// Error response
    Error {
        /// JSON-RPC version (always "2.0")
        jsonrpc: String,
        /// Request ID (null if unknown)
        id: serde_json::Value,
        /// Error details
        error: JsonRpcError,
    },
}

impl McpResponse {
    /// Create a success response
    pub fn success(result: serde_json::Value, id: Option<serde_json::Value>) -> Self {
        McpResponse::Success {
            jsonrpc: "2.0".to_string(),
            id: id.unwrap_or(serde_json::Value::Null),
            result,
        }
    }

    /// Create an error response
    pub fn error(code: i32, message: &str, id: Option<serde_json::Value>) -> Self {
        McpResponse::Error {
            jsonrpc: "2.0".to_string(),
            id: id.unwrap_or(serde_json::Value::Null),
            error: JsonRpcError {
                code,
                message: message.to_string(),
                data: None,
            },
        }
    }

    /// Create an error response with data
    pub fn error_with_data(
        code: i32,
        message: &str,
        data: serde_json::Value,
        id: Option<serde_json::Value>,
    ) -> Self {
        McpResponse::Error {
            jsonrpc: "2.0".to_string(),
            id: id.unwrap_or(serde_json::Value::Null),
            error: JsonRpcError {
                code,
                message: message.to_string(),
                data: Some(data),
            },
        }
    }
}

/// JSON-RPC 2.0 Error object
#[derive(Debug, Clone, Serialize)]
pub struct JsonRpcError {
    /// Error code
    pub code: i32,
    /// Error message
    pub message: String,
    /// Optional additional data
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

// ============================================================
// MCP-specific Parameter Types
// ============================================================

/// Parameters for tools/call method
#[derive(Debug, Clone, Deserialize)]
pub struct ToolsCallParams {
    /// Tool name
    pub name: String,
    /// Tool arguments
    #[serde(default)]
    pub arguments: Option<serde_json::Value>,
}

/// Parameters for prompts/get method
#[derive(Debug, Clone, Deserialize)]
pub struct PromptsGetParams {
    /// Prompt name
    pub name: String,
    /// Prompt arguments
    #[serde(default)]
    pub arguments: Option<serde_json::Value>,
}

/// Parameters for resources/read method
#[derive(Debug, Clone, Deserialize)]
pub struct ResourcesReadParams {
    /// Resource URI
    pub uri: String,
}

// ============================================================
// MCP Initialize Response Types
// ============================================================

/// Result of initialize method
#[derive(Debug, Clone, Serialize)]
pub struct InitializeResult {
    /// Protocol version
    #[serde(rename = "protocolVersion")]
    pub protocol_version: String,
    /// Server capabilities
    pub capabilities: Capabilities,
    /// Server info
    #[serde(rename = "serverInfo")]
    pub server_info: McpServerInfo,
    /// Optional instructions for LLM
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,
}

/// Server capabilities
#[derive(Debug, Clone, Serialize)]
pub struct Capabilities {
    /// Tools capability
    pub tools: serde_json::Value,
    /// Prompts capability
    pub prompts: serde_json::Value,
    /// Resources capability
    pub resources: serde_json::Value,
}

/// MCP Server info (for initialize response)
#[derive(Debug, Clone, Serialize)]
pub struct McpServerInfo {
    /// Server name
    pub name: String,
    /// Server version
    pub version: String,
}

// ============================================================
// Error Codes (JSON-RPC 2.0 + MCP extensions)
// ============================================================

/// Standard JSON-RPC 2.0 error codes
pub mod error_codes {
    /// Parse error (-32700)
    pub const PARSE_ERROR: i32 = -32700;
    /// Invalid request (-32600)
    pub const INVALID_REQUEST: i32 = -32600;
    /// Method not found (-32601)
    pub const METHOD_NOT_FOUND: i32 = -32601;
    /// Invalid params (-32602)
    pub const INVALID_PARAMS: i32 = -32602;
    /// Internal error (-32603)
    pub const INTERNAL_ERROR: i32 = -32603;

    /// Server not initialized (-32002) - MCP extension
    pub const SERVER_NOT_INITIALIZED: i32 = -32002;
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_mcp_request_is_notification() {
        let req = McpRequest {
            jsonrpc: Some("2.0".to_string()),
            id: None,
            method: "test".to_string(),
            params: json!({}),
        };
        assert!(req.is_notification());

        let req_with_id = McpRequest {
            jsonrpc: Some("2.0".to_string()),
            id: Some(json!(1)),
            method: "test".to_string(),
            params: json!({}),
        };
        assert!(!req_with_id.is_notification());
    }

    #[test]
    fn test_mcp_request_has_null_id() {
        let req = McpRequest {
            jsonrpc: Some("2.0".to_string()),
            id: Some(serde_json::Value::Null),
            method: "test".to_string(),
            params: json!({}),
        };
        assert!(req.has_null_id());
    }

    #[test]
    fn test_mcp_response_success() {
        let resp = McpResponse::success(json!({"result": "ok"}), Some(json!(1)));
        let serialized = serde_json::to_string(&resp).unwrap();
        assert!(serialized.contains("\"jsonrpc\":\"2.0\""));
        assert!(serialized.contains("\"id\":1"));
        assert!(serialized.contains("\"result\""));
    }

    #[test]
    fn test_mcp_response_error() {
        let resp = McpResponse::error(-32600, "Invalid request", Some(json!(1)));
        let serialized = serde_json::to_string(&resp).unwrap();
        assert!(serialized.contains("\"jsonrpc\":\"2.0\""));
        assert!(serialized.contains("\"code\":-32600"));
    }
}
