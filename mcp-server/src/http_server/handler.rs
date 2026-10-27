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

    if request.is_notification_method_with_id() {
        return Some(create_error_response(
            -32600,
            &format!(
                "Invalid request: '{}' is a notification, send it without an id",
                request.method
            ),
            request.id.clone(),
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

            // Same API-key, localhost and ACL checks as the equivalent tool
            // (MCP audit 2026-10-14 M1)
            let caller = CallerContext::new(remote_addr, api_key.map(|s| s.to_string()));
            let ctx = ServiceContext::new(caller, is_initialized, None);
            if let Err(msg) = service.authorize_resource(&ctx, &params.uri) {
                return Some(create_error_response(-32001, &msg, request.id.clone()));
            }

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::pbt::{assert_answers, random_request, tool_names, Gen};

    /// P8: the HTTP handler answers any request with its id, like the stdio
    /// one, for an initialized localhost client and an external one (which
    /// the ACL denies most tools)
    #[test]
    fn test_any_request_gets_a_jsonrpc_answer_with_its_id() {
        let dir = tempfile::tempdir().unwrap();
        let adapter = Arc::new(crate::IronBaseAdapter::new(dir.path().join("h.mlite")).unwrap());
        adapter
            .insert_one("users", serde_json::json!({"name": "a", "n": 1}))
            .unwrap();
        let service = crate::IronBaseService::new(
            adapter.clone(),
            crate::AclManager::new(adapter),
            crate::ApiKeyCache::new(60, false),
            crate::ServerInfo {
                protocol: "http".into(),
                host: "localhost".into(),
                port: 0,
                require_api_key: false,
            },
            false,
            crate::scripting::LimitsManager::new(),
            None,
            None,
        );
        let local: SocketAddr = "127.0.0.1:5000".parse().unwrap();
        let external: SocketAddr = "8.8.8.8:5000".parse().unwrap();
        let initialized = Mutex::new(HashSet::new());
        for addr in [local, external] {
            super::super::mark_client_initialized(
                &initialized,
                &super::super::client_identity(None, Some(addr)),
            );
        }
        let tools = tool_names();
        let mut gen = Gen(0x2545_f491_4f6c_dd1d);
        for case in 0..crate::transport::pbt::cases() {
            let request = random_request(&mut gen, &tools);
            let addr = if case % 2 == 0 { local } else { external };
            assert_answers(case, &request, |r| {
                handle_request(
                    r,
                    &service,
                    &initialized,
                    None,
                    Some(addr),
                    Duration::from_secs(30),
                    None,
                )
            });
        }
    }
}
