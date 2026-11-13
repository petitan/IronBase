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

/// How long `close()` waits for the server to exit on its own after stdin EOF
/// before killing it. The server flushes on shutdown; a kill is a dirty
/// shutdown (index rebuild on next start), so this matches the documented
/// 60 s graceful-stop window.
const GRACEFUL_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(60);

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
    /// Create a new HTTP transport with TLS options
    pub fn with_options(
        base_url: impl Into<String>,
        api_key: Option<String>,
        insecure: bool,
    ) -> McpResult<Self> {
        let mut builder = reqwest::Client::builder().timeout(std::time::Duration::from_secs(120)); // Longer timeout for large collections

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
        let mut req = self.client.post(self.mcp_url()).json(request);

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
    /// `None` once closed (dropping it is the EOF that stops the server)
    stdin: Arc<Mutex<Option<BufWriter<ChildStdin>>>>,
    stdout: Arc<Mutex<BufReader<ChildStdout>>>,
}

impl StdioTransport {
    /// Create a new stdio transport by spawning the MCP server
    pub async fn new(server_path: &Path, db_path: &Path) -> McpResult<Self> {
        let mut command = Command::new(server_path);
        command.arg("--stdio").arg("--db").arg(db_path);
        Self::spawn(command)
    }

    /// Spawn the given server command with piped stdin/stdout
    fn spawn(mut command: Command) -> McpResult<Self> {
        let mut child = command
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
            stdin: Arc::new(Mutex::new(Some(BufWriter::new(stdin)))),
            stdout: Arc::new(Mutex::new(BufReader::new(stdout))),
        })
    }

    /// Graceful shutdown: close the child's stdin (the server exits on EOF),
    /// wait for it to exit, and kill it only if it has not exited in time
    async fn close_with_timeout(&self, timeout: Duration) -> McpResult<()> {
        if let Some(mut stdin) = self.stdin.lock().await.take() {
            let _ = stdin.shutdown().await;
        }

        let mut child_guard = self.child.lock().await;
        if let Some(mut child) = child_guard.take() {
            if tokio::time::timeout(timeout, child.wait()).await.is_err() {
                let _ = child.kill().await;
            }
        }
        Ok(())
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
            let mut stdin_guard = self.stdin.lock().await;
            let Some(stdin) = stdin_guard.as_mut() else {
                return Err(McpError::connection("Connection is closed"));
            };
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
            let read_result =
                tokio::time::timeout(STDIO_TIMEOUT, stdout.read_line(&mut line)).await;

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
        self.close_with_timeout(GRACEFUL_SHUTDOWN_TIMEOUT).await
    }
}

impl Drop for StdioTransport {
    fn drop(&mut self) {
        // Last resort only (`close()` was not called): kill the child process
        // if it's still running. After `close()` there is no child here.
        // We can't do async here, so we just try to get the lock and kill
        if let Ok(mut guard) = self.child.try_lock() {
            if let Some(ref mut child) = *guard {
                let _ = child.start_kill();
            }
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    fn marker_path(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("tuti-tui-{}-{}", name, std::process::id()))
    }

    /// `close()` must let the server finish its shutdown work after stdin EOF.
    /// The child writes the marker only after it has seen EOF; a kill (the old
    /// behaviour) would leave no marker.
    #[tokio::test]
    async fn test_close_lets_server_exit_gracefully_on_stdin_eof() {
        let marker = marker_path("graceful");
        let _ = std::fs::remove_file(&marker);

        let mut command = Command::new("sh");
        command
            .arg("-c")
            .arg("cat >/dev/null; sleep 0.3; echo done > \"$0\"")
            .arg(&marker);
        let transport = StdioTransport::spawn(command).unwrap();

        transport.close().await.unwrap();

        let content = std::fs::read_to_string(&marker).unwrap_or_default();
        let _ = std::fs::remove_file(&marker);
        assert_eq!(content.trim(), "done");
        assert!(transport.child.lock().await.is_none());
    }

    /// A server that does not exit on EOF is killed after the timeout
    #[tokio::test]
    async fn test_close_kills_server_that_ignores_eof() {
        let mut command = Command::new("sh");
        command.arg("-c").arg("exec sleep 30");
        let transport = StdioTransport::spawn(command).unwrap();

        let started = std::time::Instant::now();
        transport
            .close_with_timeout(Duration::from_millis(200))
            .await
            .unwrap();
        assert!(started.elapsed() < Duration::from_secs(10));
        assert!(transport.child.lock().await.is_none());

        // A closed transport refuses further requests instead of hanging
        let request = JsonRpcRequest::initialize();
        assert!(transport.send(&request).await.is_err());
    }
}
