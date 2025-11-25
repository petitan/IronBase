#!/usr/bin/env python3
"""
MCP DOCJL STDIO Wrapper

This script wraps the HTTP-based MCP DOCJL server to make it compatible
with Claude Code, which expects stdio-based MCP servers.

It reads JSON-RPC requests from stdin, forwards them to the HTTP server,
and writes responses to stdout.
"""

import sys
import json
import requests
from typing import Dict, Any

# Server configuration
SERVER_URL = "http://127.0.0.1:8080/mcp"
API_KEY = "dev_key_12345"  # Change this to match your config.toml


def log_error(message: str):
    """Log errors to stderr"""
    print(f"[MCP STDIO Wrapper] {message}", file=sys.stderr, flush=True)


def forward_request(request: Dict[str, Any]) -> Dict[str, Any]:
    """Forward JSON-RPC request to HTTP server"""
    try:
        response = requests.post(
            SERVER_URL,
            json=request,
            headers={
                "Content-Type": "application/json",
                "Authorization": f"Bearer {API_KEY}",
            },
            timeout=30,
        )
        response.raise_for_status()
        return response.json()
    except requests.RequestException as e:
        log_error(f"HTTP request failed: {e}")
        return {
            "jsonrpc": "2.0",
            "id": request.get("id"),
            "error": {
                "code": -32603,
                "message": f"Internal error: {str(e)}",
            },
        }


def main():
    """Main loop: read from stdin, forward to HTTP server, write to stdout"""
    log_error("MCP STDIO Wrapper started")
    log_error(f"Forwarding requests to {SERVER_URL}")

    # Process each line from stdin
    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue

        try:
            # Parse JSON-RPC request
            request = json.loads(line)
            log_error(f"Received request: {request.get('method', 'unknown')}")

            # Forward to HTTP server
            response = forward_request(request)

            # Write response to stdout
            print(json.dumps(response), flush=True)
            log_error(f"Sent response for request ID: {request.get('id')}")

        except json.JSONDecodeError as e:
            log_error(f"Invalid JSON: {e}")
            error_response = {
                "jsonrpc": "2.0",
                "id": None,
                "error": {"code": -32700, "message": "Parse error"},
            }
            print(json.dumps(error_response), flush=True)
        except Exception as e:
            log_error(f"Unexpected error: {e}")
            error_response = {
                "jsonrpc": "2.0",
                "id": None,
                "error": {"code": -32603, "message": f"Internal error: {str(e)}"},
            }
            print(json.dumps(error_response), flush=True)


if __name__ == "__main__":
    main()
