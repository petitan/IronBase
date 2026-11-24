//! MCP Client - high-level API for IronBase MCP operations

use crate::logging::{redact_arguments, sanitize_for_log};
use crate::mcp::error::{McpError, McpResult};
use crate::mcp::protocol::{JsonRpcRequest, JsonRpcResponse, ToolCallResult};
use crate::mcp::transport::{HttpTransport, StdioTransport, Transport};
use ironbase_mcp_types::Tool;
use serde_json::Value;
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

/// Log how a request to the server ended: what was called, how long it took
/// and, on failure, the (sanitised) error text
fn log_outcome<T>(what: &str, tool: &str, started: Instant, result: &McpResult<T>) {
    let duration_ms = started.elapsed().as_millis() as u64;
    match result {
        Ok(_) => tracing::info!(tool, duration_ms, outcome = "ok", "{}", what),
        Err(e) => tracing::warn!(
            tool,
            duration_ms,
            outcome = "error",
            error = %sanitize_for_log(&e.to_string()),
            "{}",
            what
        ),
    }
}

/// The JSON-RPC error of a request sent before `initialize`: the server does
/// not know this client (it restarted and lost its sessions)
const NOT_INITIALIZED: i64 = -32002;

/// MCP Client for IronBase operations (a clone shares the transport)
#[derive(Clone)]
pub struct McpClient {
    transport: Arc<dyn Transport>,
    initialized: bool,
}

impl McpClient {
    /// Connect via HTTP transport with TLS options
    pub async fn connect_http_with_options(
        url: &str,
        api_key: Option<String>,
        insecure: bool,
    ) -> McpResult<Self> {
        let transport = HttpTransport::with_options(url, api_key, insecure)?;
        let mut client = Self {
            transport: Arc::new(transport),
            initialized: false,
        };
        client.initialize().await?;
        Ok(client)
    }

    /// Client over a ready transport (tests: no connection needed)
    #[cfg(test)]
    pub(crate) fn with_transport(transport: Arc<dyn Transport>) -> Self {
        Self {
            transport,
            initialized: true,
        }
    }

    /// Connect via stdio transport (spawns MCP server)
    pub async fn connect_stdio(server_path: &Path, db_path: &Path) -> McpResult<Self> {
        let transport = StdioTransport::new(server_path, db_path).await?;
        let mut client = Self {
            transport: Arc::new(transport),
            initialized: false,
        };
        client.initialize().await?;
        Ok(client)
    }

    /// Initialize the MCP connection
    async fn initialize(&mut self) -> McpResult<()> {
        self.handshake().await?;
        self.initialized = true;
        Ok(())
    }

    /// The MCP `initialize` exchange
    async fn handshake(&self) -> McpResult<()> {
        let started = Instant::now();
        let request = JsonRpcRequest::initialize();
        let result = match self.transport.send(&request).await {
            Ok(response) if response.error.is_some() => {
                Err(McpError::invalid_response("Initialize failed"))
            }
            Ok(_) => Ok(()),
            Err(e) => Err(e),
        };
        log_outcome("MCP initialize", "initialize", started, &result);
        result
    }

    /// Call an MCP tool and get the result. Every call is logged: name,
    /// duration and outcome at info level, the (redacted) arguments only at
    /// debug level - the result is never logged.
    async fn call_tool(&self, name: &str, arguments: Value) -> McpResult<Value> {
        tracing::debug!(
            tool = name,
            arguments = %sanitize_for_log(&redact_arguments(&arguments).to_string()),
            "MCP tool call started"
        );
        let started = Instant::now();
        let result = self.call_tool_unlogged(name, arguments).await;
        log_outcome("MCP tool call", name, started, &result);
        result
    }

    async fn call_tool_unlogged(&self, name: &str, arguments: Value) -> McpResult<Value> {
        if !self.initialized {
            return Err(McpError::NotInitialized);
        }

        let request = JsonRpcRequest::tool_call(name, arguments.clone());
        let mut response = self.send_cancellable(&request).await?;

        // The server restarted and forgot this client: initialize again and
        // repeat the request once
        if response
            .error
            .as_ref()
            .is_some_and(|e| e.code == NOT_INITIALIZED)
        {
            tracing::info!(
                tool = name,
                "server does not know this session: initializing again"
            );
            self.handshake().await?;
            let request = JsonRpcRequest::tool_call(name, arguments);
            response = self.send_cancellable(&request).await?;
        }

        // Check for RPC error
        if let Some(error) = response.error {
            return Err(McpError::rpc(error.code, error.message));
        }

        // Parse tool result
        let result = response
            .result
            .ok_or_else(|| McpError::invalid_response("Missing result in response"))?;

        // Parse as ToolCallResult
        let tool_result: ToolCallResult = serde_json::from_value(result)?;

        if tool_result.is_error {
            let msg = tool_result.text().unwrap_or("Unknown error");
            return Err(McpError::tool(msg));
        }

        // Parse text content as JSON
        let text = tool_result.text().unwrap_or("null");
        let value: Value = serde_json::from_str(text)?;

        Ok(value)
    }

    /// Send a request; if the caller stops waiting for it (a cancelled
    /// background load), the server is told to stop working on it
    async fn send_cancellable(&self, request: &JsonRpcRequest) -> McpResult<JsonRpcResponse> {
        let guard = CancelOnDrop {
            transport: Some(self.transport.clone()),
            id: request.id,
        };
        let response = self.transport.send(request).await;
        guard.disarm();
        response
    }

    /// Call the tool `T` with typed arguments. The result must parse as
    /// `T::Resp`: a missing or wrong-typed field is an error, never a default.
    pub async fn call<T: Tool>(&self, args: &T::Args) -> McpResult<T::Resp> {
        let result = self.call_tool(T::NAME, serde_json::to_value(args)?).await?;
        serde_json::from_value(result)
            .map_err(|e| McpError::invalid_response(format!("{}: {}", T::NAME, e)))
    }

    /// Close the connection
    pub async fn close(&self) -> McpResult<()> {
        self.transport.close().await
    }

    // === High-level database operations ===

    /// Get server info (version, etc.)
    pub async fn server_info(&self) -> McpResult<Value> {
        // Try to get URL from transport and fetch /health endpoint
        if let Some(url) = self.transport.get_base_url() {
            let health_url = format!("{}/health", url.trim_end_matches("/mcp"));
            let client = reqwest::Client::new();
            if let Ok(response) = client.get(&health_url).send().await {
                if let Ok(json) = response.json::<Value>().await {
                    return Ok(json);
                }
            }
        }
        // Fallback: return empty object
        Ok(serde_json::json!({"version": "unknown"}))
    }

    /// Get list of available MCP tools
    pub async fn tools_list(&self) -> McpResult<Vec<Value>> {
        if !self.initialized {
            return Err(McpError::NotInitialized);
        }

        let request = JsonRpcRequest::tools_list();
        let response = self.transport.send(&request).await?;

        if let Some(error) = response.error {
            return Err(McpError::rpc(error.code, error.message));
        }

        let result = response
            .result
            .ok_or_else(|| McpError::invalid_response("Missing result in response"))?;

        // Extract tools array
        let tools: Vec<Value> = result
            .get("tools")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();

        Ok(tools)
    }

    /// Get list of available MCP prompts
    pub async fn prompts_list(&self) -> McpResult<Vec<Value>> {
        if !self.initialized {
            return Err(McpError::NotInitialized);
        }

        let request = JsonRpcRequest::new("prompts/list", None);
        let response = self.transport.send(&request).await?;

        if let Some(error) = response.error {
            return Err(McpError::rpc(error.code, error.message));
        }

        let result = response
            .result
            .ok_or_else(|| McpError::invalid_response("Missing result in response"))?;

        // Extract prompts array
        let prompts: Vec<Value> = result
            .get("prompts")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();

        Ok(prompts)
    }
}

impl Drop for McpClient {
    fn drop(&mut self) {
        // Best effort close - we can't do async in drop
        // The transport's Drop impl will handle cleanup
    }
}

/// Sends `notifications/cancelled` for a request whose answer nobody waits
/// for any more (its future was dropped before the answer came)
struct CancelOnDrop {
    transport: Option<Arc<dyn Transport>>,
    id: u64,
}

impl CancelOnDrop {
    /// The answer came: nothing to cancel
    fn disarm(mut self) {
        self.transport = None;
    }
}

impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        let Some(transport) = self.transport.take() else {
            return;
        };
        let id = self.id;
        // Without a runtime (shutdown) there is no one to send it
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            runtime.spawn(async move {
                let params =
                    serde_json::json!({"requestId": id, "reason": "cancelled by the user"});
                if let Err(e) = transport.notify("notifications/cancelled", params).await {
                    tracing::debug!(error = %e, request = id, "could not send the cancellation");
                }
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use std::sync::Mutex;

    /// A server that restarted: it refuses tool calls with `refusal` until
    /// the client initializes again
    struct RestartedServer {
        refusal: i64,
        initialized: Mutex<bool>,
        methods: Mutex<Vec<String>>,
    }

    impl RestartedServer {
        fn refusing_with(refusal: i64) -> Arc<Self> {
            Arc::new(Self {
                refusal,
                initialized: Mutex::new(false),
                methods: Mutex::new(Vec::new()),
            })
        }
    }

    #[async_trait]
    impl Transport for RestartedServer {
        async fn send(&self, request: &JsonRpcRequest) -> McpResult<JsonRpcResponse> {
            self.methods.lock().unwrap().push(request.method.clone());
            let mut initialized = self.initialized.lock().unwrap();
            let body = match request.method.as_str() {
                "initialize" => {
                    *initialized = true;
                    serde_json::json!({"result": {}})
                }
                _ if !*initialized => serde_json::json!({"error": {
                    "code": self.refusal,
                    "message": "refused"
                }}),
                _ => serde_json::json!({"result": {"content": [
                    {"type": "text", "text": "{\"collections\": [\"users\"]}"}
                ]}}),
            };
            let mut response = serde_json::json!({"jsonrpc": "2.0", "id": request.id});
            response
                .as_object_mut()
                .unwrap()
                .extend(body.as_object().unwrap().clone());
            Ok(serde_json::from_value(response).unwrap())
        }

        async fn close(&self) -> McpResult<()> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn test_a_restarted_server_is_initialized_again() {
        let server = RestartedServer::refusing_with(NOT_INITIALIZED);
        let client = McpClient::with_transport(server.clone());

        let collections = client
            .call::<ironbase_mcp_types::tools::CollectionList>(
                &ironbase_mcp_types::params::NoParams {},
            )
            .await
            .unwrap()
            .collections;

        assert_eq!(collections, vec!["users".to_string()]);
        assert_eq!(
            *server.methods.lock().unwrap(),
            vec!["tools/call", "initialize", "tools/call"]
        );
    }

    /// Another JSON-RPC error is reported as it is, without a new handshake
    #[tokio::test]
    async fn test_other_rpc_errors_are_not_retried() {
        let server = RestartedServer::refusing_with(-32602);
        let client = McpClient::with_transport(server.clone());

        let error = client
            .call_tool("x", serde_json::json!({}))
            .await
            .unwrap_err();

        assert!(
            matches!(error, McpError::Rpc { code: -32602, .. }),
            "{}",
            error
        );
        assert_eq!(*server.methods.lock().unwrap(), vec!["tools/call"]);
    }

    /// A server that answers a tool call only when told to, and records the
    /// notifications it gets
    struct SlowServer {
        answer: tokio::sync::Notify,
        notifications: Mutex<Vec<(String, Value)>>,
    }

    #[async_trait]
    impl Transport for SlowServer {
        async fn send(&self, request: &JsonRpcRequest) -> McpResult<JsonRpcResponse> {
            self.answer.notified().await;
            let text = serde_json::json!({"ok": true}).to_string();
            Ok(serde_json::from_value(serde_json::json!({
                "jsonrpc": "2.0",
                "id": request.id,
                "result": {"content": [{"type": "text", "text": text}]}
            }))?)
        }

        async fn notify(&self, method: &str, params: Value) -> McpResult<()> {
            self.notifications
                .lock()
                .unwrap()
                .push((method.to_string(), params));
            Ok(())
        }

        async fn close(&self) -> McpResult<()> {
            Ok(())
        }
    }

    /// A call nobody waits for any more tells the server to stop it (the
    /// server could not tell before, and kept working on a cancelled load);
    /// an answered call sends nothing
    #[tokio::test]
    async fn test_a_dropped_call_is_cancelled_on_the_server() {
        let server = Arc::new(SlowServer {
            answer: tokio::sync::Notify::new(),
            notifications: Mutex::new(Vec::new()),
        });
        let client = McpClient::with_transport(server.clone());

        let pending = tokio::time::timeout(
            std::time::Duration::from_millis(20),
            client.call_tool("find", serde_json::json!({})),
        )
        .await;
        assert!(pending.is_err(), "the server did not answer");
        // The notification is sent by a task of its own
        for _ in 0..10 {
            tokio::task::yield_now().await;
        }
        let notifications = server.notifications.lock().unwrap().clone();
        assert_eq!(notifications.len(), 1);
        assert_eq!(notifications[0].0, "notifications/cancelled");
        assert!(notifications[0].1["requestId"].is_u64());

        server.answer.notify_one();
        client
            .call_tool("find", serde_json::json!({}))
            .await
            .unwrap();
        for _ in 0..10 {
            tokio::task::yield_now().await;
        }
        assert_eq!(server.notifications.lock().unwrap().len(), 1);
    }
}
