//! MCP Client - high-level API for IronBase MCP operations

use crate::logging::{redact_arguments, sanitize_for_log};
use crate::mcp::error::{McpError, McpResult};
use crate::mcp::protocol::{JsonRpcRequest, ToolCallResult};
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

/// MCP Client for IronBase operations
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
        result?;

        self.initialized = true;
        Ok(())
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

        let request = JsonRpcRequest::tool_call(name, arguments);
        let response = self.transport.send(&request).await?;

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
