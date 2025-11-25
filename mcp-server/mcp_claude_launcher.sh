#!/bin/bash
#
# MCP DOCJL Server Launcher for Claude Code
#
# This script launches the MCP DOCJL HTTP server (if not already running)
# and starts the stdio wrapper for Claude Code integration.
#

set -e

# Configuration
SERVER_DIR="/home/petitan/MongoLite/mcp-server"
SERVER_BIN="$SERVER_DIR/target/release/mcp-docjl-server"
CONFIG_FILE="$SERVER_DIR/config.toml"
WRAPPER_SCRIPT="$SERVER_DIR/mcp_bridge.py"
PID_FILE="/tmp/mcp-docjl-server.pid"
SERVER_URL="http://127.0.0.1:8080/health"

cd "$SERVER_DIR"

# Function to check if server is running
is_server_running() {
    curl -s -f "$SERVER_URL" > /dev/null 2>&1
    return $?
}

# Function to start the HTTP server
start_http_server() {
    echo "[Launcher] Starting MCP DOCJL HTTP server..." >&2

    # Export config path
    export DOCJL_CONFIG="$CONFIG_FILE"

    # Start server in background
    nohup "$SERVER_BIN" > /tmp/mcp-docjl-server.log 2>&1 &
    echo $! > "$PID_FILE"

    # Wait for server to be ready
    for i in {1..10}; do
        if is_server_running; then
            echo "[Launcher] HTTP server started successfully (PID: $(cat $PID_FILE))" >&2
            return 0
        fi
        echo "[Launcher] Waiting for server to start... ($i/10)" >&2
        sleep 1
    done

    echo "[Launcher] ERROR: Server failed to start" >&2
    return 1
}

# Check if server is already running
if ! is_server_running; then
    echo "[Launcher] HTTP server not running, starting it..." >&2
    if ! start_http_server; then
        echo "[Launcher] FATAL: Could not start HTTP server" >&2
        exit 1
    fi
else
    echo "[Launcher] HTTP server already running" >&2
fi

# Start the stdio wrapper
echo "[Launcher] Starting stdio wrapper..." >&2
exec python3 "$WRAPPER_SCRIPT"
