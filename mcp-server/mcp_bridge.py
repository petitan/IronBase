#!/usr/bin/env python3
"""
MCP Bridge for Windows Claude Desktop -> WSL HTTP Server

This script acts as a simple STDIO <-> HTTP proxy between Claude Desktop (Windows)
and the MCP DOCJL Server running in WSL via HTTP.

All MCP protocol logic is handled by the Rust server.

Usage:
    python mcp_bridge.py

Claude Desktop Configuration (Windows):
    {
      "mcpServers": {
        "docjl-editor": {
          "command": "python",
          "args": ["C:\\path\\to\\mcp_bridge.py"]
        }
      }
    }
"""

import sys
import json
import requests
from typing import Dict, Any

# Configuration
MCP_SERVER_URL = "http://localhost:8080/mcp"
DEBUG = False  # Set to True for debugging

def log_debug(message: str):
    """Log debug message to stderr (won't interfere with stdout)"""
    if DEBUG:
        print(f"[MCP Bridge] {message}", file=sys.stderr, flush=True)
        # Also log to file for debugging on Windows
        try:
            with open("C:\\Users\\Kalman\\Desktop\\mcp_bridge_debug.log", "a", encoding="utf-8") as f:
                from datetime import datetime
                timestamp = datetime.now().strftime("%Y-%m-%d %H:%M:%S")
                f.write(f"[{timestamp}] {message}\n")
        except:
            pass  # Ignore file logging errors

def process_request(request_line: str) -> Dict[str, Any]:
    """
    Process a single JSON-RPC request from stdin

    Forwards all requests to the Rust HTTP server unchanged.
    The Rust server handles all MCP protocol logic.

    Args:
        request_line: JSON-RPC request string

    Returns:
        JSON-RPC response dict
    """
    try:
        # Parse incoming JSON-RPC request
        request = json.loads(request_line)
        method = request.get("method", "unknown")
        request_id = request.get("id")

        log_debug(f"Forwarding request: {method} (id: {request_id})")

        # Forward to Rust HTTP server (handles all MCP protocol logic)
        response = requests.post(
            MCP_SERVER_URL,
            json=request,
            headers={"Content-Type": "application/json"},
            timeout=30  # 30 second timeout
        )

        # Parse and return response unchanged
        if response.status_code == 200:
            result = response.json()
            log_debug(f"Response successful for {method}")
            return result
        else:
            log_debug(f"HTTP error {response.status_code} for {method}")
            return {
                "jsonrpc": "2.0",
                "id": request_id if request_id is not None else -1,
                "error": {
                    "code": -32603,
                    "message": f"HTTP error {response.status_code}: {response.text}"
                }
            }

    except json.JSONDecodeError as e:
        log_debug(f"JSON decode error: {e}")
        return {
            "jsonrpc": "2.0",
            "id": -1,  # Use -1 for parse errors (no valid request ID available)
            "error": {
                "code": -32700,
                "message": f"Parse error: {str(e)}"
            }
        }
    except requests.exceptions.ConnectionError:
        log_debug("Connection error - is WSL server running?")
        return {
            "jsonrpc": "2.0",
            "id": request.get("id") if 'request' in locals() else -1,
            "error": {
                "code": -32603,
                "message": "Cannot connect to WSL server at http://localhost:8080. Is the server running?"
            }
        }
    except requests.exceptions.Timeout:
        log_debug("Request timeout")
        return {
            "jsonrpc": "2.0",
            "id": request.get("id") if 'request' in locals() else -1,
            "error": {
                "code": -32603,
                "message": "Request timeout (30s)"
            }
        }
    except Exception as e:
        log_debug(f"Unexpected error: {e}")
        return {
            "jsonrpc": "2.0",
            "id": request.get("id") if 'request' in locals() else -1,
            "error": {
                "code": -32603,
                "message": f"Internal error: {str(e)}"
            }
        }

def main():
    """
    Main loop: Read JSON-RPC from stdin, forward to HTTP server, write response to stdout
    """
    log_debug("MCP Bridge starting (dumb proxy mode)...")
    log_debug(f"Target server: {MCP_SERVER_URL}")

    # Test connection to server
    try:
        response = requests.get("http://localhost:8080/health", timeout=5)
        if response.status_code == 200:
            log_debug("✅ WSL server is reachable")
        else:
            log_debug(f"⚠️  WSL server returned status {response.status_code}")
    except Exception as e:
        log_debug(f"⚠️  Cannot reach WSL server: {e}")
        log_debug("Make sure the server is running in WSL:")
        log_debug("  cd /home/petitan/MongoLite/mcp-server")
        log_debug("  DOCJL_CONFIG=config.toml ./target/release/mcp-docjl-server")

    # Main loop: read from stdin, process, write to stdout
    log_debug("Entering main loop (waiting for stdin)...")

    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue

        log_debug(f"Processing request ({len(line)} bytes)")

        # Process request (simple forward to HTTP server)
        response = process_request(line)

        # Write response to stdout
        response_json = json.dumps(response, ensure_ascii=False)
        print(response_json, flush=True)
        log_debug(f"Sent response ({len(response_json)} bytes)")

if __name__ == "__main__":
    try:
        main()
    except KeyboardInterrupt:
        log_debug("Bridge interrupted by user")
        sys.exit(0)
    except Exception as e:
        log_debug(f"Fatal error: {e}")
        sys.exit(1)
