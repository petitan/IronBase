//! MCP request handler

use super::{
    client_identity, create_error_response, create_success_response, get_server_instructions,
    is_client_initialized, mark_client_initialized,
};
use crate::acl::CallerContext;
use crate::transport::{
    Capabilities, InitializeResult, McpRequest, McpResponse, McpServerInfo, PromptsGetParams,
    ToolsCallParams,
};
use crate::VERSION;
use std::collections::HashSet;
use std::net::SocketAddr;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Handle MCP request using the service layer
///
/// This function handles MCP protocol routing and delegates tool execution
/// to the IronBaseService for authentication, authorization, and dispatch.
pub(crate) fn handle_request(
    request: &McpRequest,
    service: &crate::IronBaseService,
    initialized_clients: &Mutex<HashSet<String>>,
    api_key: Option<&str>,
    remote_addr: Option<SocketAddr>,
    tool_timeout: Duration,
    cancel_flag: Option<Arc<AtomicBool>>,
) -> Option<McpResponse> {
    use crate::{
        get_prompt_content, get_prompts_list, get_resources_list, get_tools_list_filtered,
        read_resource, ServiceContext, ToolRequest,
    };
    let has_null_id = matches!(&request.id, Some(v) if v.is_null());
    let is_notification = request.id.is_none();
    let client_id = client_identity(api_key, remote_addr);
    let is_initialized = is_client_initialized(initialized_clients, &client_id);

    if has_null_id {
        return Some(create_error_response(
            -32600,
            "Invalid request: id must be omitted for notifications",
            Some(serde_json::Value::Null),
        ));
    }

    // MCP lifecycle enforcement: only allow initialize and ping before initialization
    // Per spec: "The initialization phase MUST be the first interaction"
    if !is_initialized
        && request.method != "initialize"
        && request.method != "ping"
        && !request.method.starts_with("notifications/")
    {
        if is_notification {
            return None;
        }
        return Some(create_error_response(
            -32002, // Server not initialized (custom error code)
            "Server not initialized. Call 'initialize' first.",
            request.id.clone(),
        ));
    }

    match request.method.as_str() {
        "initialize" => {
            if is_notification {
                return None;
            }
            mark_client_initialized(initialized_clients, &client_id);
            let init_result = InitializeResult {
                protocol_version: "2025-06-18".to_string(),
                capabilities: Capabilities {
                    tools: serde_json::json!({"listChanged": false}),
                    prompts: serde_json::json!({"listChanged": false}),
                    resources: serde_json::json!({"subscribe": false, "listChanged": true}),
                },
                server_info: McpServerInfo {
                    name: "ironbase-mcp".to_string(),
                    version: VERSION.to_string(),
                },
                instructions: Some(get_server_instructions()),
            };
            match serde_json::to_value(init_result) {
                Ok(value) => Some(create_success_response(value, request.id.clone())),
                Err(e) => {
                    tracing::error!("Failed to serialize initialize response: {}", e);
                    Some(create_error_response(
                        -32603,
                        &format!("Internal error: failed to serialize response: {}", e),
                        request.id.clone(),
                    ))
                }
            }
        }

        "initialized" | "notifications/initialized" => None,

        "ping" => {
            if is_notification {
                return None;
            }
            Some(create_success_response(
                serde_json::json!({}),
                request.id.clone(),
            ))
        }

        // notifications/cancelled is handled in the HTTP layer before spawn_blocking
        // to ensure immediate cancellation without blocking on the tool execution
        "notifications/cancelled" => None,

        "tools/list" => {
            // SECURITY FIX #14: Filter admin tools for non-localhost callers
            let is_localhost = remote_addr
                .map(|addr| {
                    crate::InterfaceType::from_socket_addr(addr) == crate::InterfaceType::Localhost
                })
                .unwrap_or(false);
            if is_notification {
                return None;
            }
            Some(create_success_response(
                get_tools_list_filtered(is_localhost),
                request.id.clone(),
            ))
        }

        "tools/call" => {
            let params: ToolsCallParams = match serde_json::from_value(request.params.clone()) {
                Ok(p) => p,
                Err(e) => {
                    if is_notification {
                        return None;
                    }
                    return Some(create_error_response(
                        -32602,
                        &format!("Invalid params: {}", e),
                        request.id.clone(),
                    ));
                }
            };

            let arguments = params.arguments.unwrap_or_else(|| serde_json::json!({}));

            // Create service context with cancellation support
            let caller = CallerContext::new(remote_addr, api_key.map(|s| s.to_string()));
            let deadline = Some(Instant::now() + tool_timeout);
            let ctx = if let Some(flag) = cancel_flag {
                ServiceContext::with_cancel_flag(caller, is_initialized, deadline, flag)
            } else {
                ServiceContext::new(caller, is_initialized, deadline)
            };

            // Create tool request
            let tool_request = ToolRequest::new(&params.name, arguments);

            // Execute via service layer (handles auth, ACL, dispatch)
            match service.execute_tool(&ctx, &tool_request) {
                crate::ToolResult::Success(result) => {
                    if is_notification {
                        return None;
                    }
                    let response = serde_json::json!({
                        "content": [{
                            "type": "text",
                            "text": serde_json::to_string_pretty(&result).unwrap_or_else(|_| "{}".to_string())
                        }]
                    });
                    Some(create_success_response(response, request.id.clone()))
                }
                crate::ToolResult::Error { code, message } => {
                    if is_notification {
                        return None;
                    }
                    // MCP spec: Tool errors MUST be returned as success with isError: true
                    // JSON-RPC errors are only for protocol-level errors (parse, method not found, etc.)
                    // See: https://modelcontextprotocol.io/specification/2025-06-18/schema
                    let response = serde_json::json!({
                        "content": [{
                            "type": "text",
                            "text": format!("[Error {}] {}", code, message)
                        }],
                        "isError": true
                    });
                    Some(create_success_response(response, request.id.clone()))
                }
                crate::ToolResult::AccessDenied(msg) => {
                    if is_notification {
                        return None;
                    }
                    let response = serde_json::json!({
                        "content": [{
                            "type": "text",
                            "text": msg
                        }],
                        "isError": true
                    });
                    Some(create_success_response(response, request.id.clone()))
                }
            }
        }

        "prompts/list" => Some(create_success_response(
            {
                if is_notification {
                    return None;
                }
                get_prompts_list()
            },
            request.id.clone(),
        )),

        "prompts/get" => {
            if is_notification {
                return None;
            }
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

        "resources/list" => Some(create_success_response(
            {
                if is_notification {
                    return None;
                }
                get_resources_list(service.adapter())
            },
            request.id.clone(),
        )),

        "resources/read" => {
            if is_notification {
                return None;
            }
            #[derive(serde::Deserialize)]
            struct ResourcesReadParams {
                uri: String,
            }

            let params: ResourcesReadParams = match serde_json::from_value(request.params.clone()) {
                Ok(p) => p,
                Err(e) => {
                    return Some(create_error_response(
                        -32602,
                        &format!("Invalid params: {}", e),
                        request.id.clone(),
                    ));
                }
            };

            match read_resource(service.adapter(), &params.uri) {
                Ok(content) => Some(create_success_response(content, request.id.clone())),
                Err(e) => Some(create_error_response(-32602, &e, request.id.clone())),
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
