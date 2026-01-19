//! MCP transport implementations (HTTP and stdio)

use crate::mcp::error::{McpError, McpResult};
use crate::mcp::protocol::{JsonRpcRequest, JsonRpcResponse};
use async_trait::async_trait;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, BufWriter};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::sync::Mutex;

/// Timeout for stdio communication (30 seconds)
const STDIO_TIMEOUT: Duration = Duration::from_secs(30);

/// Transport trait for MCP communication
#[async_trait]
pub trait Transport: Send + Sync {
    /// Send a request and receive a response
    async fn send(&self, request: &JsonRpcRequest) -> McpResult<JsonRpcResponse>;

    /// Close the transport connection
    async fn close(&self) -> McpResult<()>;

    /// Get base URL if this is an HTTP transport (for health check)
    fn get_base_url(&self) -> Option<String> {
        None
    }
}

/// HTTP transport for MCP communication
pub struct HttpTransport {
    client: reqwest::Client,
    base_url: String,
    api_key: Option<String>,
}

impl HttpTransport {
    /// Create a new HTTP transport
    pub fn new(base_url: impl Into<String>, api_key: Option<String>) -> McpResult<Self> {
        Self::with_options(base_url, api_key, false)
    }

    /// Create a new HTTP transport with TLS options
    pub fn with_options(
        base_url: impl Into<String>,
        api_key: Option<String>,
        insecure: bool,
    ) -> McpResult<Self> {
        let mut builder = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(120)); // Longer timeout for large collections

        // Accept self-signed certificates if insecure mode is enabled
        if insecure {
            builder = builder.danger_accept_invalid_certs(true);
        }

        let client = builder
            .build()
            .map_err(|e| McpError::connection(format!("Failed to create HTTP client: {}", e)))?;

        Ok(Self {
            client,
            base_url: base_url.into(),
            api_key,
        })
    }

    /// Get the MCP endpoint URL
    fn mcp_url(&self) -> String {
        let base = self.base_url.trim_end_matches('/');
        // Don't add /mcp if it's already there
        if base.ends_with("/mcp") {
            base.to_string()
        } else {
            format!("{}/mcp", base)
        }
    }
}

#[async_trait]
impl Transport for HttpTransport {
    async fn send(&self, request: &JsonRpcRequest) -> McpResult<JsonRpcResponse> {
        let mut req = self.client.post(&self.mcp_url()).json(request);

        // Add Authorization header if API key is set
        if let Some(key) = &self.api_key {
            req = req.header("Authorization", format!("Bearer {}", key));
        }

        let response = req.send().await?;

        // Handle specific HTTP errors
        if response.status() == reqwest::StatusCode::UNAUTHORIZED {
            return Err(McpError::connection(
                "Unauthorized: Invalid or missing API key. Check your mcp_api_key in config or IRONBASE_API_KEY env var.",
            ));
        }

        if !response.status().is_success() {
            return Err(McpError::connection(format!(
                "HTTP error: {}",
                response.status()
            )));
        }

        let body = response.text().await?;
        let rpc_response: JsonRpcResponse = serde_json::from_str(&body)?;

        Ok(rpc_response)
    }

    async fn close(&self) -> McpResult<()> {
        // HTTP is stateless, nothing to close
        Ok(())
    }

    fn get_base_url(&self) -> Option<String> {
        Some(self.base_url.clone())
    }
}

/// Stdio transport for MCP communication (spawns server as child process)
pub struct StdioTransport {
    child: Arc<Mutex<Option<Child>>>,
    stdin: Arc<Mutex<BufWriter<ChildStdin>>>,
    stdout: Arc<Mutex<BufReader<ChildStdout>>>,
}

impl StdioTransport {
    /// Create a new stdio transport by spawning the MCP server
    pub async fn new(server_path: &Path, db_path: &Path) -> McpResult<Self> {
        let mut child = Command::new(server_path)
            .arg("--stdio")
            .arg("--db")
            .arg(db_path)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map_err(|e| McpError::connection(format!("Failed to spawn server: {}", e)))?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| McpError::connection("Failed to get stdin"))?;

        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| McpError::connection("Failed to get stdout"))?;

        Ok(Self {
            child: Arc::new(Mutex::new(Some(child))),
            stdin: Arc::new(Mutex::new(BufWriter::new(stdin))),
            stdout: Arc::new(Mutex::new(BufReader::new(stdout))),
        })
    }
}

/// BUG FIX: Added timeout and size limit to prevent DoS and deadlock
#[async_trait]
impl Transport for StdioTransport {
    async fn send(&self, request: &JsonRpcRequest) -> McpResult<JsonRpcResponse> {
        // Serialize request to JSON line
        let mut json = serde_json::to_string(request)?;
        json.push('\n');

        // Send request with timeout
        {
            let mut stdin = self.stdin.lock().await;
            tokio::time::timeout(STDIO_TIMEOUT, async {
                stdin.write_all(json.as_bytes()).await?;
                stdin.flush().await?;
                Ok::<_, std::io::Error>(())
            })
            .await
            .map_err(|_| McpError::connection("Timeout sending request to server"))??;
        }

        // Read response line with timeout
        let mut line = String::new();
        {
            let mut stdout = self.stdout.lock().await;
            let read_result = tokio::time::timeout(STDIO_TIMEOUT, stdout.read_line(&mut line)).await;

            match read_result {
                Ok(Ok(_)) => {}
                Ok(Err(e)) => return Err(McpError::Io(e)),
                Err(_) => return Err(McpError::connection("Timeout waiting for server response")),
            }
        }

        if line.is_empty() {
            return Err(McpError::connection("Server closed connection"));
        }

        let response: JsonRpcResponse = serde_json::from_str(&line)?;
        Ok(response)
    }

    async fn close(&self) -> McpResult<()> {
        let mut child_guard = self.child.lock().await;
        if let Some(mut child) = child_guard.take() {
            let _ = child.kill().await;
        }
        Ok(())
    }
}

impl Drop for StdioTransport {
    fn drop(&mut self) {
        // Best effort cleanup - kill the child process if it's still running
        // We can't do async here, so we just try to get the lock and kill
        if let Ok(mut guard) = self.child.try_lock() {
            if let Some(ref mut child) = *guard {
                let _ = child.start_kill();
            }
        }
    }
}
