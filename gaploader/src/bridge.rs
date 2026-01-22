//! Bridge client - spawns ironbase-bridge and communicates via JSON-RPC over stdio

use crate::error::{GaploaderError, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};

/// JSON-RPC 2.0 request
#[derive(Debug, Serialize)]
struct JsonRpcRequest {
    jsonrpc: &'static str,
    id: u64,
    method: String,
    params: serde_json::Value,
}

/// JSON-RPC 2.0 response
#[derive(Debug, Deserialize)]
struct JsonRpcResponse {
    #[allow(dead_code)]
    jsonrpc: String,
    #[allow(dead_code)]
    id: Option<serde_json::Value>,
    result: Option<serde_json::Value>,
    error: Option<JsonRpcError>,
}

/// JSON-RPC 2.0 error
#[derive(Debug, Deserialize)]
struct JsonRpcError {
    code: i32,
    message: String,
    #[allow(dead_code)]
    data: Option<serde_json::Value>,
}

/// Bridge client that spawns ironbase-bridge as a child process
pub struct BridgeClient {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    request_id: AtomicU64,
}

impl BridgeClient {
    /// Spawn ironbase-bridge with given configuration
    pub async fn spawn(
        bridge_path: &Path,
        server_url: &str,
        api_key: Option<&str>,
        insecure: bool,
    ) -> Result<Self> {
        let mut cmd = Command::new(bridge_path);
        cmd.arg("--server").arg(server_url);

        if let Some(key) = api_key {
            cmd.arg("--api-key").arg(key);
        }

        if insecure {
            cmd.arg("--insecure");
        }

        // Disable health check retries for faster startup
        cmd.arg("--health-retries").arg("1");

        cmd.stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::inherit()); // Pass stderr through for debugging

        let mut child = cmd.spawn().map_err(|e| {
            GaploaderError::BridgeSpawnFailed(format!(
                "Failed to spawn {}: {}",
                bridge_path.display(),
                e
            ))
        })?;

        let stdin = child.stdin.take().ok_or_else(|| {
            GaploaderError::BridgeSpawnFailed("Failed to capture stdin".to_string())
        })?;

        let stdout = child.stdout.take().ok_or_else(|| {
            GaploaderError::BridgeSpawnFailed("Failed to capture stdout".to_string())
        })?;

        Ok(Self {
            child,
            stdin,
            stdout: BufReader::new(stdout),
            request_id: AtomicU64::new(1),
        })
    }

    /// Send a JSON-RPC request and wait for response
    pub async fn call(&mut self, method: &str, params: serde_json::Value) -> Result<serde_json::Value> {
        let id = self.request_id.fetch_add(1, Ordering::SeqCst);

        let request = JsonRpcRequest {
            jsonrpc: "2.0",
            id,
            method: method.to_string(),
            params,
        };

        // Serialize and send
        let json = serde_json::to_string(&request)?;
        self.stdin
            .write_all(json.as_bytes())
            .await
            .map_err(|e| GaploaderError::BridgeError(format!("Write failed: {}", e)))?;
        self.stdin
            .write_all(b"\n")
            .await
            .map_err(|e| GaploaderError::BridgeError(format!("Write newline failed: {}", e)))?;
        self.stdin
            .flush()
            .await
            .map_err(|e| GaploaderError::BridgeError(format!("Flush failed: {}", e)))?;

        // Read response
        let mut line = String::new();
        self.stdout
            .read_line(&mut line)
            .await
            .map_err(|e| GaploaderError::BridgeError(format!("Read failed: {}", e)))?;

        if line.is_empty() {
            return Err(GaploaderError::BridgeError(
                "Bridge closed connection unexpectedly".to_string(),
            ));
        }

        // Parse response
        let response: JsonRpcResponse = serde_json::from_str(&line)?;

        if let Some(error) = response.error {
            return Err(GaploaderError::JsonRpc {
                code: error.code,
                message: error.message,
            });
        }

        Ok(response.result.unwrap_or(serde_json::Value::Null))
    }

    /// Call MCP tool
    pub async fn call_tool(
        &mut self,
        tool_name: &str,
        arguments: serde_json::Value,
    ) -> Result<serde_json::Value> {
        let params = serde_json::json!({
            "name": tool_name,
            "arguments": arguments
        });

        let result = self.call("tools/call", params).await?;

        // MCP responses wrap content in a specific format
        // Extract the actual result from content[0].text
        if let Some(content) = result.get("content") {
            if let Some(first) = content.get(0) {
                if let Some(text) = first.get("text") {
                    if let Some(text_str) = text.as_str() {
                        // Parse the text as JSON
                        return serde_json::from_str(text_str).map_err(|e| {
                            GaploaderError::BridgeError(format!(
                                "Failed to parse tool result for '{}': {} (raw text: {:?})",
                                tool_name,
                                e,
                                if text_str.len() > 200 { &text_str[..200] } else { text_str }
                            ))
                        });
                    }
                }
            }
        }

        // Return as-is if not in expected format
        Ok(result)
    }

    /// Graceful shutdown
    pub async fn close(mut self) -> Result<()> {
        // Close stdin to signal EOF
        drop(self.stdin);

        // Wait for child to exit with timeout
        tokio::select! {
            result = self.child.wait() => {
                result.map_err(|e| GaploaderError::BridgeError(format!("Wait failed: {}", e)))?;
            }
            _ = tokio::time::sleep(std::time::Duration::from_secs(5)) => {
                // Force kill if not exited
                let _ = self.child.kill().await;
            }
        }

        Ok(())
    }
}
