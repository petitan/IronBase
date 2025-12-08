//! MCP transport implementations (HTTP and stdio)

use crate::mcp::error::{McpError, McpResult};
use crate::mcp::protocol::{JsonRpcRequest, JsonRpcResponse};
use async_trait::async_trait;
use std::path::Path;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, BufWriter};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::sync::Mutex;

/// Transport trait for MCP communication
#[async_trait]
pub trait Transport: Send + Sync {
    /// Send a request and receive a response
    async fn send(&self, request: &JsonRpcRequest) -> McpResult<JsonRpcResponse>;

    /// Close the transport connection
    async fn close(&self) -> McpResult<()>;
}

/// HTTP transport for MCP communication
pub struct HttpTransport {
    client: reqwest::Client,
    base_url: String,
}

impl HttpTransport {
    /// Create a new HTTP transport
    pub fn new(base_url: impl Into<String>) -> Self {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(120)) // Longer timeout for large collections
            .build()
            .expect("Failed to create HTTP client");

        Self {
            client,
            base_url: base_url.into(),
        }
    }

    /// Get the MCP endpoint URL
    fn mcp_url(&self) -> String {
        format!("{}/mcp", self.base_url.trim_end_matches('/'))
    }
}

#[async_trait]
impl Transport for HttpTransport {
    async fn send(&self, request: &JsonRpcRequest) -> McpResult<JsonRpcResponse> {
        let response = self
            .client
            .post(&self.mcp_url())
            .json(request)
            .send()
            .await?;

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

#[async_trait]
impl Transport for StdioTransport {
    async fn send(&self, request: &JsonRpcRequest) -> McpResult<JsonRpcResponse> {
        // Serialize request to JSON line
        let mut json = serde_json::to_string(request)?;
        json.push('\n');

        // Send request
        {
            let mut stdin = self.stdin.lock().await;
            stdin.write_all(json.as_bytes()).await?;
            stdin.flush().await?;
        }

        // Read response line
        let mut line = String::new();
        {
            let mut stdout = self.stdout.lock().await;
            stdout.read_line(&mut line).await?;
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
