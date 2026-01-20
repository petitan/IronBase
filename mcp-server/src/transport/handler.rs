//! Common MCP request handler logic
//!
//! This module provides the core request handling logic that is shared
//! between different transport implementations (stdio, HTTP).

use crate::transport::{
    error_codes, Capabilities, InitializeResult, McpRequest, McpResponse, McpServerInfo,
    PromptsGetParams, ResourcesReadParams, ToolsCallParams,
};
use crate::{
    dispatch_tool, get_prompt_content, get_prompts_list, get_resources_list,
    get_tools_list_filtered, read_resource, EmbeddingManager, IronBaseAdapter, VERSION,
};
use serde_json::json;
use std::sync::Arc;

/// Handler context for processing MCP requests
pub struct HandlerContext {
    /// Database adapter
    pub adapter: Arc<IronBaseAdapter>,
    /// Embedding manager (optional - may not be configured)
    pub embedding_manager: Option<Arc<EmbeddingManager>>,
    /// Whether the server has been initialized
    pub initialized: bool,
    /// Whether this is a localhost connection (affects tool visibility)
    pub is_localhost: bool,
}

impl HandlerContext {
    /// Create a new handler context
    pub fn new(adapter: Arc<IronBaseAdapter>, is_localhost: bool) -> Self {
        Self {
            adapter,
            embedding_manager: None,
            initialized: false,
            is_localhost,
        }
    }

    /// Create a new handler context with embedding manager
    pub fn with_embedding(
        adapter: Arc<IronBaseAdapter>,
        embedding_manager: Option<Arc<EmbeddingManager>>,
        is_localhost: bool,
    ) -> Self {
        Self {
            adapter,
            embedding_manager,
            initialized: false,
            is_localhost,
        }
    }

    /// Handle an MCP request and return the response
    ///
    /// Returns None for notifications (which don't get a response per JSON-RPC spec)
    pub fn handle_request(&mut self, request: &McpRequest) -> Option<McpResponse> {
        // Check for invalid null id
        if request.has_null_id() {
            return Some(McpResponse::error(
                error_codes::INVALID_REQUEST,
                "Invalid request: id must be omitted for notifications",
                Some(serde_json::Value::Null),
            ));
        }

        let is_notification = request.is_notification();

        // MCP lifecycle enforcement: only allow initialize and ping before initialization
        if !self.initialized
            && request.method != "initialize"
            && request.method != "ping"
            && !request.method.starts_with("notifications/")
        {
            if is_notification {
                return None;
            }
            return Some(McpResponse::error(
                error_codes::SERVER_NOT_INITIALIZED,
                "Server not initialized. Call 'initialize' first.",
                request.id.clone(),
            ));
        }

        match request.method.as_str() {
            "initialize" => self.handle_initialize(request, is_notification),
            "initialized" | "notifications/initialized" => {
                // Notification - no response
                None
            }
            "ping" => self.handle_ping(request, is_notification),
            "notifications/cancelled" => None,
            "tools/list" => self.handle_tools_list(request, is_notification),
            "tools/call" => self.handle_tools_call(request, is_notification),
            "prompts/list" => self.handle_prompts_list(request, is_notification),
            "prompts/get" => self.handle_prompts_get(request, is_notification),
            "resources/list" => self.handle_resources_list(request, is_notification),
            "resources/read" => self.handle_resources_read(request, is_notification),
            _ if is_notification => None,
            _ => Some(McpResponse::error(
                error_codes::METHOD_NOT_FOUND,
                &format!("Method not found: {}", request.method),
                request.id.clone(),
            )),
        }
    }

    fn handle_initialize(
        &mut self,
        request: &McpRequest,
        is_notification: bool,
    ) -> Option<McpResponse> {
        if is_notification {
            return None;
        }

        self.initialized = true;

        let result = InitializeResult {
            protocol_version: "2025-06-18".to_string(),
            capabilities: Capabilities {
                tools: json!({"listChanged": false}),
                prompts: json!({"listChanged": false}),
                resources: json!({"subscribe": false, "listChanged": true}),
            },
            server_info: McpServerInfo {
                name: "ironbase-mcp".to_string(),
                version: VERSION.to_string(),
            },
            instructions: None,
        };

        match serde_json::to_value(result) {
            Ok(value) => Some(McpResponse::success(value, request.id.clone())),
            Err(e) => Some(McpResponse::error(
                error_codes::INTERNAL_ERROR,
                &format!("Serialization error: {}", e),
                request.id.clone(),
            )),
        }
    }

    fn handle_ping(&self, request: &McpRequest, is_notification: bool) -> Option<McpResponse> {
        if is_notification {
            return None;
        }
        Some(McpResponse::success(json!({}), request.id.clone()))
    }

    fn handle_tools_list(
        &self,
        request: &McpRequest,
        is_notification: bool,
    ) -> Option<McpResponse> {
        if is_notification {
            return None;
        }
        Some(McpResponse::success(
            get_tools_list_filtered(self.is_localhost),
            request.id.clone(),
        ))
    }

    fn handle_tools_call(
        &self,
        request: &McpRequest,
        is_notification: bool,
    ) -> Option<McpResponse> {
        let params: ToolsCallParams = match serde_json::from_value(request.params.clone()) {
            Ok(p) => p,
            Err(e) => {
                if is_notification {
                    return None;
                }
                return Some(McpResponse::error(
                    error_codes::INVALID_PARAMS,
                    &format!("Invalid params: {}", e),
                    request.id.clone(),
                ));
            }
        };

        let arguments = params.arguments.unwrap_or_else(|| json!({}));

        // Note: stdio transport doesn't support cancellation (None cancel_flag)
        match dispatch_tool(
            &params.name,
            arguments,
            &self.adapter,
            None,
            None,
            None,
            None,
            &self.embedding_manager,
        ) {
            Ok(result) => {
                if is_notification {
                    return None;
                }
                let response = json!({
                    "content": [{
                        "type": "text",
                        "text": serde_json::to_string_pretty(&result).unwrap_or_else(|_| "{}".to_string())
                    }]
                });
                Some(McpResponse::success(response, request.id.clone()))
            }
            Err(e) => {
                if is_notification {
                    return None;
                }
                // MCP spec: Tool errors MUST be returned as success with isError: true
                // JSON-RPC errors are only for protocol-level errors (parse, method not found, etc.)
                // See: https://modelcontextprotocol.io/specification/2025-06-18/schema
                let response = json!({
                    "content": [{
                        "type": "text",
                        "text": format!("[Error {}] {}", e.code.code(), e)
                    }],
                    "isError": true
                });
                Some(McpResponse::success(response, request.id.clone()))
            }
        }
    }

    fn handle_prompts_list(
        &self,
        request: &McpRequest,
        is_notification: bool,
    ) -> Option<McpResponse> {
        if is_notification {
            return None;
        }
        Some(McpResponse::success(get_prompts_list(), request.id.clone()))
    }

    fn handle_prompts_get(
        &self,
        request: &McpRequest,
        is_notification: bool,
    ) -> Option<McpResponse> {
        if is_notification {
            return None;
        }

        let params: PromptsGetParams = match serde_json::from_value(request.params.clone()) {
            Ok(p) => p,
            Err(e) => {
                return Some(McpResponse::error(
                    error_codes::INVALID_PARAMS,
                    &format!("Invalid params: {}", e),
                    request.id.clone(),
                ));
            }
        };

        let arguments = params.arguments.unwrap_or_else(|| json!({}));

        match get_prompt_content(&params.name, &arguments) {
            Some(content) => Some(McpResponse::success(content, request.id.clone())),
            None => Some(McpResponse::error(
                error_codes::INVALID_PARAMS,
                &format!("Prompt '{}' not found", params.name),
                request.id.clone(),
            )),
        }
    }

    fn handle_resources_list(
        &self,
        request: &McpRequest,
        is_notification: bool,
    ) -> Option<McpResponse> {
        if is_notification {
            return None;
        }
        Some(McpResponse::success(
            get_resources_list(&self.adapter),
            request.id.clone(),
        ))
    }

    fn handle_resources_read(
        &self,
        request: &McpRequest,
        is_notification: bool,
    ) -> Option<McpResponse> {
        if is_notification {
            return None;
        }

        let params: ResourcesReadParams = match serde_json::from_value(request.params.clone()) {
            Ok(p) => p,
            Err(e) => {
                return Some(McpResponse::error(
                    error_codes::INVALID_PARAMS,
                    &format!("Invalid params: {}", e),
                    request.id.clone(),
                ));
            }
        };

        match read_resource(&self.adapter, &params.uri) {
            Ok(content) => Some(McpResponse::success(content, request.id.clone())),
            Err(e) => Some(McpResponse::error(
                error_codes::INVALID_PARAMS,
                &e,
                request.id.clone(),
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEST_COUNTER: AtomicU64 = AtomicU64::new(0);

    fn create_test_adapter() -> Arc<IronBaseAdapter> {
        let temp_dir = std::env::temp_dir();
        let counter = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
        let db_path = temp_dir.join(format!(
            "test_handler_{}_{}.mlite",
            std::process::id(),
            counter
        ));
        Arc::new(IronBaseAdapter::new(db_path.to_string_lossy().to_string()).unwrap())
    }

    #[test]
    fn test_handle_ping_before_initialize() {
        let adapter = create_test_adapter();
        let mut ctx = HandlerContext::new(adapter, true);

        let request = McpRequest {
            jsonrpc: Some("2.0".to_string()),
            id: Some(json!(1)),
            method: "ping".to_string(),
            params: json!({}),
        };

        let response = ctx.handle_request(&request);
        assert!(response.is_some());
    }

    #[test]
    fn test_handle_initialize() {
        let adapter = create_test_adapter();
        let mut ctx = HandlerContext::new(adapter, true);

        let request = McpRequest {
            jsonrpc: Some("2.0".to_string()),
            id: Some(json!(1)),
            method: "initialize".to_string(),
            params: json!({}),
        };

        let response = ctx.handle_request(&request);
        assert!(response.is_some());
        assert!(ctx.initialized);
    }

    #[test]
    fn test_reject_before_initialize() {
        let adapter = create_test_adapter();
        let mut ctx = HandlerContext::new(adapter, true);

        let request = McpRequest {
            jsonrpc: Some("2.0".to_string()),
            id: Some(json!(1)),
            method: "tools/list".to_string(),
            params: json!({}),
        };

        let response = ctx.handle_request(&request);
        assert!(response.is_some());

        // Should be an error response
        if let McpResponse::Error { error, .. } = response.unwrap() {
            assert_eq!(error.code, error_codes::SERVER_NOT_INITIALIZED);
        } else {
            panic!("Expected error response");
        }
    }
}
