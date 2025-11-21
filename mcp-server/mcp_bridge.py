#!/usr/bin/env python3
"""
MCP Bridge for Windows Claude Desktop -> WSL HTTP Server

This script acts as a bridge between Claude Desktop (Windows) and the
MCP DOCJL Server running in WSL via HTTP.

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

def handle_mcp_protocol(request: Dict[str, Any]) -> Dict[str, Any]:
    """
    Handle MCP protocol messages (initialize, tools/list, tools/call)

    Returns MCP protocol response directly without forwarding to server
    """
    method = request.get("method")
    request_id = request.get("id")

    # Handle initialize - MCP handshake
    if method == "initialize":
        log_debug("Handling MCP initialize")
        return {
            "jsonrpc": "2.0",
            "id": request_id,
            "result": {
                "protocolVersion": "2025-06-18",
                "capabilities": {
                    "tools": {}
                },
                "serverInfo": {
                    "name": "docjl-editor",
                    "version": "0.1.0"
                }
            }
        }

    # Handle tools/list - List available MCP tools
    elif method == "tools/list":
        log_debug("Handling tools/list")
        return {
            "jsonrpc": "2.0",
            "id": request_id,
            "result": {
                "tools": [
                    {
                        "name": "mcp_docjl_list_documents",
                        "description": "List all DOCJL documents",
                        "inputSchema": {
                            "type": "object",
                            "properties": {
                                "filter": {"type": "object", "description": "Optional filter"}
                            }
                        }
                    },
                    {
                        "name": "mcp_docjl_get_document",
                        "description": "Get full DOCJL document by ID",
                        "inputSchema": {
                            "type": "object",
                            "properties": {
                                "document_id": {"type": "string", "description": "Document ID"}
                            },
                            "required": ["document_id"]
                        }
                    },
                    {
                        "name": "mcp_docjl_list_headings",
                        "description": "Get document outline/table of contents",
                        "inputSchema": {
                            "type": "object",
                            "properties": {
                                "document_id": {"type": "string", "description": "Document ID"}
                            },
                            "required": ["document_id"]
                        }
                    },
                    {
                        "name": "mcp_docjl_search_blocks",
                        "description": "Search for blocks in documents",
                        "inputSchema": {
                            "type": "object",
                            "properties": {
                                "document_id": {"type": "string", "description": "Document ID"},
                                "query": {"type": "object", "description": "Search query"}
                            },
                            "required": ["document_id", "query"]
                        }
                    },
                    {
                        "name": "mcp_docjl_insert_block",
                        "description": "Insert new content block into document",
                        "inputSchema": {
                            "type": "object",
                            "properties": {
                                "document_id": {"type": "string"},
                                "block": {"type": "object"},
                                "position": {"type": "string"}
                            },
                            "required": ["document_id", "block"]
                        }
                    },
                    {
                        "name": "mcp_docjl_update_block",
                        "description": "Update existing block",
                        "inputSchema": {
                            "type": "object",
                            "properties": {
                                "document_id": {"type": "string"},
                                "block_label": {"type": "string"},
                                "updates": {"type": "object"}
                            },
                            "required": ["document_id", "block_label", "updates"]
                        }
                    },
                    {
                        "name": "mcp_docjl_delete_block",
                        "description": "Delete block from document",
                        "inputSchema": {
                            "type": "object",
                            "properties": {
                                "document_id": {"type": "string"},
                                "block_label": {"type": "string"}
                            },
                            "required": ["document_id", "block_label"]
                        }
                    }
                ]
            }
        }

    # Handle tools/call - Execute a tool
    elif method == "tools/call":
        log_debug("Handling tools/call")
        params = request.get("params", {})
        tool_name = params.get("name")
        tool_arguments = params.get("arguments", {})

        # Forward to backend as direct method call
        backend_request = {
            "jsonrpc": "2.0",
            "method": tool_name,
            "params": tool_arguments,
            "id": request_id
        }

        try:
            response = requests.post(
                MCP_SERVER_URL,
                json=backend_request,
                headers={"Content-Type": "application/json"},
                timeout=30
            )

            if response.status_code == 200:
                backend_result = response.json()

                # Wrap backend result in MCP tools/call response format
                if "result" in backend_result:
                    return {
                        "jsonrpc": "2.0",
                        "id": request_id,
                        "result": {
                            "content": [
                                {
                                    "type": "text",
                                    "text": json.dumps(backend_result["result"], indent=2)
                                }
                            ]
                        }
                    }
                elif "error" in backend_result:
                    # Backend returned error - ensure proper format
                    return {
                        "jsonrpc": "2.0",
                        "id": request_id,
                        "error": backend_result["error"]
                    }
                else:
                    # Unknown backend response
                    return {
                        "jsonrpc": "2.0",
                        "id": request_id,
                        "error": {
                            "code": -32603,
                            "message": f"Unexpected backend response: {json.dumps(backend_result)}"
                        }
                    }
            else:
                return {
                    "jsonrpc": "2.0",
                    "id": request_id,
                    "error": {
                        "code": -32603,
                        "message": f"Backend error {response.status_code}: {response.text}"
                    }
                }
        except Exception as e:
            return {
                "jsonrpc": "2.0",
                "id": request_id,
                "error": {
                    "code": -32603,
                    "message": f"Tool execution error: {str(e)}"
                }
            }

    # Unknown MCP method
    else:
        return None  # Not an MCP protocol message

def process_request(request_line: str) -> Dict[str, Any]:
    """
    Process a single JSON-RPC request from stdin

    Args:
        request_line: JSON-RPC request string

    Returns:
        JSON-RPC response dict
    """
    try:
        # Parse incoming JSON-RPC request
        request = json.loads(request_line)
        log_debug(f"Received request: {request.get('method', 'unknown')}")

        # Check if this is an MCP protocol message
        mcp_response = handle_mcp_protocol(request)
        if mcp_response is not None:
            log_debug(f"Handled as MCP protocol message")
            return mcp_response

        # Not MCP protocol - forward to WSL HTTP server
        response = requests.post(
            MCP_SERVER_URL,
            json=request,
            headers={"Content-Type": "application/json"},
            timeout=30  # 30 second timeout
        )

        # Parse response
        if response.status_code == 200:
            result = response.json()
            log_debug(f"Response successful")
            return result
        else:
            log_debug(f"HTTP error: {response.status_code}")
            return {
                "jsonrpc": "2.0",
                "id": request.get("id") if request.get("id") is not None else -1,
                "error": {
                    "code": -32603,
                    "message": f"HTTP error {response.status_code}: {response.text}"
                }
            }

    except json.JSONDecodeError as e:
        log_debug(f"JSON decode error: {e}")
        return {
            "jsonrpc": "2.0",
            "id": -1,  # Use -1 instead of None for parse errors (no valid request ID available)
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
    log_debug("MCP Bridge starting...")
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

        # Process request
        response = process_request(line)

        # Write response to stdout
        response_json = json.dumps(response)
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
