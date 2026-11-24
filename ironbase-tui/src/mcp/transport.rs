//! MCP transport implementations (HTTP and stdio)

use crate::mcp::error::{McpError, McpResult};
use crate::mcp::protocol::{JsonRpcRequest, JsonRpcResponse};
use async_trait::async_trait;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWriteExt, BufReader, BufWriter};
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

    /// Send a notification (no answer). Only HTTP sends one; stdio serves
    /// one request at a time, so there is nothing a notification could stop
    async fn notify(&self, _method: &str, _params: serde_json::Value) -> McpResult<()> {
        Ok(())
    }

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

        // A timeout (504) or a failed request (500) comes with a JSON-RPC
        // error body naming the cause: that is the answer, not the bare status
        let status = response.status();
        let body = response.text().await?;
        match (
            status.is_success(),
            serde_json::from_str::<JsonRpcResponse>(&body),
        ) {
            (true, parsed) => Ok(parsed?),
            (false, Ok(rpc_response)) if rpc_response.error.is_some() => Ok(rpc_response),
            // A non-2xx answer without a JSON-RPC error (a proxy, a crash page)
            (false, _) => Err(McpError::connection(format!("HTTP error: {}", status))),
        }
    }

    async fn notify(&self, method: &str, params: serde_json::Value) -> McpResult<()> {
        let notification =
            serde_json::json!({"jsonrpc": "2.0", "method": method, "params": params});
        let mut req = self.client.post(self.mcp_url()).json(&notification);
        if let Some(key) = &self.api_key {
            req = req.header("Authorization", format!("Bearer {}", key));
        }
        let response = req.send().await?;
        if !response.status().is_success() {
            return Err(McpError::connection(format!(
                "HTTP error: {}",
                response.status()
            )));
        }
        Ok(())
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
    /// The server's pipes, under one lock: a request and the reading of its
    /// answer are one exchange
    io: Arc<Mutex<StdioPipes>>,
}

struct StdioPipes {
    /// `None` once closed (dropping it is the EOF that stops the server)
    stdin: Option<BufWriter<ChildStdin>>,
    stdout: BufReader<ChildStdout>,
}

/// Read lines from `reader` until the JSON-RPC response to request `id`.
/// An error without an id (the server could not tell or answer the request,
/// JSON-RPC's `"id": null`) is the answer too: one exchange runs at a time.
/// Any other line is skipped: the late answer to a request that timed out
/// (which would otherwise be read as the answer to the next one), a
/// notification, or output that is not a response.
async fn read_response<R: AsyncBufRead + Unpin>(
    reader: &mut R,
    id: u64,
) -> McpResult<JsonRpcResponse> {
    let mut line = String::new();
    loop {
        line.clear();
        if reader.read_line(&mut line).await? == 0 {
            return Err(McpError::connection("Server closed connection"));
        }
        match serde_json::from_str::<JsonRpcResponse>(&line) {
            Ok(response) if response.id == Some(id) => return Ok(response),
            Ok(response) if response.id.is_none() && response.error.is_some() => {
                return Ok(response)
            }
            Ok(response) => tracing::debug!(
                expected = id,
                got = ?response.id,
                "skipping a response to another request"
            ),
            Err(_) => tracing::debug!(expected = id, "skipping a line that is not a response"),
        }
    }
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
            io: Arc::new(Mutex::new(StdioPipes {
                stdin: Some(BufWriter::new(stdin)),
                stdout: BufReader::new(stdout),
            })),
        })
    }

    /// Graceful shutdown: close the child's stdin (the server exits on EOF),
    /// wait for it to exit, and kill it only if it has not exited in time
    async fn close_with_timeout(&self, timeout: Duration) -> McpResult<()> {
        if let Some(mut stdin) = self.io.lock().await.stdin.take() {
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

#[async_trait]
impl Transport for StdioTransport {
    async fn send(&self, request: &JsonRpcRequest) -> McpResult<JsonRpcResponse> {
        // Serialize request to JSON line
        let mut json = serde_json::to_string(request)?;
        json.push('\n');

        // The exchange runs as a task of its own: a caller that stops waiting
        // (a cancelled load) must not cut a request line in half or leave its
        // answer half read - the next request would be garbled or misread
        let io = self.io.clone();
        let id = request.id;
        tokio::spawn(async move {
            let mut io = io.lock().await;
            let StdioPipes { stdin, stdout } = &mut *io;
            let Some(stdin) = stdin.as_mut() else {
                return Err(McpError::connection("Connection is closed"));
            };

            // Send request with timeout
            tokio::time::timeout(STDIO_TIMEOUT, async {
                stdin.write_all(json.as_bytes()).await?;
                stdin.flush().await?;
                Ok::<_, std::io::Error>(())
            })
            .await
            .map_err(|_| McpError::connection("Timeout sending request to server"))??;

            // Read this request's response with timeout
            tokio::time::timeout(STDIO_TIMEOUT, read_response(stdout, id))
                .await
                .map_err(|_| McpError::connection("Timeout waiting for server response"))?
        })
        .await
        .map_err(|e| McpError::connection(format!("Request to the server failed: {e}")))?
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

#[cfg(test)]
mod read_tests {
    use super::*;

    /// The late answer to a timed-out request, a notification and a stray
    /// line come before this request's answer: the answer is the one with its id
    #[tokio::test]
    async fn test_read_response_skips_other_requests_answers() {
        let lines = concat!(
            r#"{"jsonrpc":"2.0","id":6,"result":{"stale":true}}"#,
            "\n",
            r#"{"jsonrpc":"2.0","method":"notifications/progress"}"#,
            "\n",
            "not json\n",
            r#"{"jsonrpc":"2.0","id":7,"result":{"mine":true}}"#,
            "\n",
        );
        let mut reader = tokio::io::BufReader::new(lines.as_bytes());

        let response = read_response(&mut reader, 7).await.unwrap();

        assert_eq!(response.result, Some(serde_json::json!({"mine": true})));
    }

    /// The server's answer when it cannot serialize its response carries no
    /// id: it is this request's error, not a line to wait past
    #[tokio::test]
    async fn test_read_response_takes_an_error_without_id() {
        let lines = concat!(
            r#"{"jsonrpc":"2.0","id":6,"result":{"stale":true}}"#,
            "\n",
            r#"{"jsonrpc":"2.0","id":null,"error":{"code":-32603,"message":"Internal error"}}"#,
            "\n",
        );
        let mut reader = tokio::io::BufReader::new(lines.as_bytes());

        let response = read_response(&mut reader, 7).await.unwrap();

        assert_eq!(response.error.unwrap().code, -32603);
    }

    #[tokio::test]
    async fn test_read_response_reports_a_closed_pipe() {
        let lines = concat!(r#"{"jsonrpc":"2.0","id":6,"result":{}}"#, "\n");
        let mut reader = tokio::io::BufReader::new(lines.as_bytes());

        let error = read_response(&mut reader, 7).await.unwrap_err();

        assert!(error.to_string().contains("closed"), "{}", error);
    }

    /// Serve one raw HTTP response on a local port; returns the base URL
    async fn serve_once(status: &str, body: &str) -> String {
        use tokio::io::AsyncReadExt;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let response = format!(
            "HTTP/1.1 {}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
            status,
            body.len(),
            body
        );
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buf = vec![0u8; 8192];
            let _ = socket.read(&mut buf).await;
            socket.write_all(response.as_bytes()).await.unwrap();
            let _ = socket.shutdown().await;
        });
        format!("http://{}", addr)
    }

    /// The server answers a tool timeout with 504 and a JSON-RPC error that
    /// says what to do: that error reaches the caller, not "HTTP error: 504"
    #[tokio::test]
    async fn test_http_error_status_keeps_the_servers_json_rpc_error() {
        let body = r#"{"jsonrpc":"2.0","id":1,"error":{"code":-32008,"message":"Operation timed out after 25 seconds. Tool: 'find'."}}"#;
        let url = serve_once("504 Gateway Timeout", body).await;
        let transport = HttpTransport::with_options(url, None, false).unwrap();

        let response = transport.send(&JsonRpcRequest::tools_list()).await.unwrap();

        let error = response.error.unwrap();
        assert_eq!(error.code, -32008);
        assert!(error.message.contains("timed out after 25 seconds"));
    }

    /// A non-2xx answer without a JSON-RPC body is still a status error
    #[tokio::test]
    async fn test_http_error_status_without_json_rpc_body() {
        let url = serve_once("502 Bad Gateway", "<html>proxy</html>").await;
        let transport = HttpTransport::with_options(url, None, false).unwrap();

        let error = transport
            .send(&JsonRpcRequest::tools_list())
            .await
            .unwrap_err();

        assert!(error.to_string().contains("502"), "{}", error);
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

    /// A caller that stops waiting in the middle of a request (a cancelled
    /// load) does not leave half a request line behind: the next request
    /// gets its own answer. The request is larger than a pipe's buffer and
    /// the server reads it late, so the caller stops while it is being
    /// written; the server answers each line with the line's first id.
    #[tokio::test]
    async fn test_a_dropped_request_does_not_garble_the_next() {
        let mut command = Command::new("sh");
        command.arg("-c").arg(
            r#"sleep 0.5; while read -r line; do id=$(printf '%s' "$line" | sed -n 's/^{"jsonrpc":"2.0","id":\([0-9]*\),.*/\1/p'); printf '{"jsonrpc":"2.0","id":%s,"result":{}}\n' "$id"; done"#,
        );
        let transport = StdioTransport::spawn(command).unwrap();

        let big =
            JsonRpcRequest::tool_call("insert", serde_json::json!({"blob": "a".repeat(300_000)}));
        let dropped = tokio::time::timeout(Duration::from_millis(100), transport.send(&big)).await;
        assert!(dropped.is_err(), "the server reads it late");

        let next = JsonRpcRequest::tools_list();
        let response = tokio::time::timeout(Duration::from_secs(10), transport.send(&next))
            .await
            .expect("the next request is answered")
            .unwrap();
        assert_eq!(response.id, Some(next.id));
        transport
            .close_with_timeout(Duration::from_millis(500))
            .await
            .unwrap();
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
