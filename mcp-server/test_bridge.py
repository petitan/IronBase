#!/usr/bin/env python3
"""
Test MCP Bridge - Logs all incoming messages from Claude Desktop
"""
import sys
import json
from datetime import datetime

LOG_FILE = "C:\\Users\\Kalman\\Desktop\\bridge_log.txt"

def log_message(msg):
    """Log message to file with timestamp"""
    timestamp = datetime.now().strftime("%Y-%m-%d %H:%M:%S")
    with open(LOG_FILE, 'a', encoding='utf-8') as f:
        f.write(f"[{timestamp}] {msg}\n")

def main():
    log_message("=" * 80)
    log_message("Test Bridge started")
    log_message("=" * 80)

    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue

        # Log the incoming request
        log_message(f"RECEIVED: {line}")

        try:
            # Parse JSON-RPC request
            request = json.loads(line)
            method = request.get('method', 'unknown')
            request_id = request.get('id')

            log_message(f"  Method: {method}")
            log_message(f"  ID: {request_id}")
            log_message(f"  Params: {request.get('params', {})}")

            # Return a simple success response for everything
            response = {
                "jsonrpc": "2.0",
                "id": request_id,
                "result": {
                    "test": "ok",
                    "received_method": method
                }
            }

            response_json = json.dumps(response)
            log_message(f"SENDING: {response_json}")

            # Send response to stdout
            print(response_json, flush=True)

        except json.JSONDecodeError as e:
            log_message(f"ERROR: JSON decode failed - {e}")
            error_response = {
                "jsonrpc": "2.0",
                "id": -1,
                "error": {
                    "code": -32700,
                    "message": f"Parse error: {str(e)}"
                }
            }
            print(json.dumps(error_response), flush=True)
        except Exception as e:
            log_message(f"ERROR: Unexpected error - {e}")

if __name__ == "__main__":
    try:
        main()
    except KeyboardInterrupt:
        log_message("Test Bridge interrupted")
        sys.exit(0)
