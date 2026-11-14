//! JSON-RPC 2.0 protocol types for MCP communication

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::atomic::{AtomicU64, Ordering};

/// Global request ID counter
static REQUEST_ID: AtomicU64 = AtomicU64::new(1);

/// Generate a unique request ID
pub fn next_request_id() -> u64 {
    REQUEST_ID.fetch_add(1, Ordering::SeqCst)
}

/// JSON-RPC 2.0 Request
#[derive(Debug, Clone, Serialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: &'static str,
    pub id: u64,
    pub method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
}

impl JsonRpcRequest {
    /// Create a new JSON-RPC request
    pub fn new(method: impl Into<String>, params: Option<Value>) -> Self {
        Self {
            jsonrpc: "2.0",
            id: next_request_id(),
            method: method.into(),
            params,
        }
    }

    /// Create an MCP tools/call request
    pub fn tool_call(name: &str, arguments: Value) -> Self {
        Self::new(
            "tools/call",
            Some(serde_json::json!({
                "name": name,
                "arguments": arguments
            })),
        )
    }

    /// Create an MCP tools/list request
    pub fn tools_list() -> Self {
        Self::new("tools/list", None)
    }

    /// Create an MCP initialize request
    pub fn initialize() -> Self {
        Self::new(
            "initialize",
            Some(serde_json::json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": {
                    "name": "ironbase-tui",
                    "version": "1.0.0"
                }
            })),
        )
    }
}

/// JSON-RPC 2.0 Response
#[derive(Debug, Clone, Deserialize)]
pub struct JsonRpcResponse {
    // Deserialization target: part of the JSON-RPC envelope, not read by the client
    #[allow(dead_code)]
    pub jsonrpc: String,
    // Deserialization target: part of the JSON-RPC envelope, not read by the client
    #[allow(dead_code)]
    pub id: Option<u64>,
    #[serde(default)]
    pub result: Option<Value>,
    #[serde(default)]
    pub error: Option<JsonRpcError>,
}

/// JSON-RPC 2.0 Error
#[derive(Debug, Clone, Deserialize)]
pub struct JsonRpcError {
    pub code: i64,
    pub message: String,
    // Deserialization target: optional JSON-RPC error payload, not read by the client
    #[allow(dead_code)]
    #[serde(default)]
    pub data: Option<Value>,
}

impl std::fmt::Display for JsonRpcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "JSON-RPC error {}: {}", self.code, self.message)
    }
}

impl std::error::Error for JsonRpcError {}

/// MCP Tool Call Result (from tools/call response)
#[derive(Debug, Clone, Deserialize)]
pub struct ToolCallResult {
    pub content: Vec<ToolContent>,
    #[serde(default, rename = "isError")]
    pub is_error: bool,
}

/// MCP Tool Content item
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type")]
pub enum ToolContent {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(other)]
    Unknown,
}

impl ToolCallResult {
    /// Extract text content from result
    pub fn text(&self) -> Option<&str> {
        for content in &self.content {
            if let ToolContent::Text { text } = content {
                return Some(text);
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_request_serialization() {
        let req = JsonRpcRequest::tool_call("find", serde_json::json!({"collection": "test"}));
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("\"jsonrpc\":\"2.0\""));
        assert!(json.contains("\"method\":\"tools/call\""));
    }

    #[test]
    fn test_response_deserialization() {
        let json = r#"{"jsonrpc":"2.0","id":1,"result":{"content":[{"type":"text","text":"[]"}]}}"#;
        let resp: JsonRpcResponse = serde_json::from_str(json).unwrap();
        assert!(resp.error.is_none() && resp.result.is_some());
    }
}
