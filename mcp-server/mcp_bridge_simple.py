#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""
Simple MCP Bridge - No external dependencies
Bulletproof version for Windows Claude Desktop

Handles JSON-RPC 2.0 correctly:
- Requests (have id) get responses
- Notifications (no id) get NO response per spec
"""

import sys
import os

# Windows binary mode for stdout/stdin - CRITICAL for Claude Desktop
if sys.platform == "win32":
    import msvcrt
    msvcrt.setmode(sys.stdin.fileno(), os.O_BINARY)
    msvcrt.setmode(sys.stdout.fileno(), os.O_BINARY)

# Debug log file (on Windows desktop for easy access)
DEBUG_LOG = None
try:
    if sys.platform == "win32":
        DEBUG_LOG = os.path.expanduser("~\\Desktop\\mcp_bridge_debug.log")
    else:
        DEBUG_LOG = "/tmp/mcp_bridge_debug.log"
except:
    pass

def log(msg):
    """Log to file only - never to stdout/stderr"""
    if DEBUG_LOG:
        try:
            with open(DEBUG_LOG, "a", encoding="utf-8") as f:
                from datetime import datetime
                f.write(f"[{datetime.now()}] {msg}\n")
        except:
            pass

def make_error(request_id, code, message):
    """Create a valid JSON-RPC error response"""
    return {
        "jsonrpc": "2.0",
        "id": request_id if request_id is not None else 1,
        "error": {
            "code": code,
            "message": str(message)
        }
    }

def is_notification(request):
    """Check if request is a notification (no id field per JSON-RPC 2.0)"""
    if "id" not in request:
        return True
    if request.get("id") is None:
        return True
    return False

def write_output(data):
    """Write JSON output to stdout - handles Windows binary mode"""
    import json
    output = json.dumps(data, ensure_ascii=True) + "\n"
    if sys.platform == "win32":
        sys.stdout.buffer.write(output.encode('utf-8'))
        sys.stdout.buffer.flush()
    else:
        print(output.rstrip(), flush=True)

def read_line():
    """Read a line from stdin - handles Windows binary mode"""
    if sys.platform == "win32":
        line = sys.stdin.buffer.readline()
        if not line:
            return None
        return line.decode('utf-8').strip()
    else:
        line = sys.stdin.readline()
        if not line:
            return None
        return line.strip()

def main():
    import json
    import socket

    SERVER_HOST = "localhost"
    SERVER_PORT = 8080

    log("=== MCP Bridge started ===")
    log(f"Platform: {sys.platform}")
    log(f"Python: {sys.version}")
    log(f"Server: {SERVER_HOST}:{SERVER_PORT}")

    def send_request(request_data):
        """Send HTTP request via raw socket. Returns (response, is_notification_response)"""
        request_id = request_data.get("id")
        notification = is_notification(request_data)

        try:
            body = json.dumps(request_data)
            log(f"Sending {'notification' if notification else 'request'}: {body[:200]}...")

            http_request = (
                f"POST /mcp HTTP/1.1\r\n"
                f"Host: {SERVER_HOST}:{SERVER_PORT}\r\n"
                f"Content-Type: application/json\r\n"
                f"Content-Length: {len(body)}\r\n"
                f"Connection: close\r\n"
                f"\r\n"
                f"{body}"
            )

            sock = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
            sock.settimeout(30)
            sock.connect((SERVER_HOST, SERVER_PORT))
            sock.sendall(http_request.encode('utf-8'))

            response = b""
            while True:
                chunk = sock.recv(4096)
                if not chunk:
                    break
                response += chunk
            sock.close()

            response_str = response.decode('utf-8')
            log(f"Raw response: {response_str[:500]}...")

            if '\r\n\r\n' not in response_str:
                log("Invalid HTTP response - no header/body separator")
                return make_error(request_id or 1, -32603, "Invalid HTTP response"), False

            headers, body = response_str.split('\r\n\r\n', 1)

            # Check for 204 No Content (notification response)
            if "204" in headers.split('\r\n')[0]:
                log("Received 204 No Content - notification acknowledged")
                return None, True

            body = body.strip()
            if not body:
                log("Empty body - notification acknowledged")
                return None, True

            result = json.loads(body)

            if isinstance(result, dict):
                if "id" not in result and request_id is not None:
                    result["id"] = request_id
                if "jsonrpc" not in result:
                    result["jsonrpc"] = "2.0"

            log(f"Parsed response OK")
            return result, False

        except socket.timeout:
            log("Socket timeout")
            return make_error(request_id or 1, -32603, "Connection timeout"), False
        except ConnectionRefusedError:
            log("Connection refused - server not running?")
            return make_error(request_id or 1, -32603, f"Cannot connect to server at {SERVER_HOST}:{SERVER_PORT}. Is the WSL server running?"), False
        except json.JSONDecodeError as e:
            log(f"JSON decode error: {e}")
            return make_error(request_id or 1, -32603, f"Invalid JSON in response: {e}"), False
        except Exception as e:
            log(f"Exception in send_request: {type(e).__name__}: {e}")
            return make_error(request_id or 1, -32603, str(e)), False

    log("Entering main loop...")

    while True:
        try:
            line = read_line()
            if line is None:
                log("EOF - exiting")
                break
            if not line:
                continue

            log(f"Received: {line[:200]}...")

            try:
                import json
                request = json.loads(line)
                notification = is_notification(request)

                if notification:
                    log(f"Processing notification: {request.get('method', '?')}")
                else:
                    log(f"Processing request id={request.get('id')}: {request.get('method', '?')}")

                response, is_notification_resp = send_request(request)

                if notification or is_notification_resp or response is None:
                    log("Notification - no response to client")
                    continue

            except json.JSONDecodeError as e:
                log(f"JSON decode error: {e}")
                response = make_error(1, -32700, f"Parse error: {e}")
            except Exception as e:
                log(f"Unexpected error: {type(e).__name__}: {e}")
                response = make_error(1, -32603, str(e))

            if response is not None:
                try:
                    log(f"Sending response: {str(response)[:200]}...")
                    write_output(response)
                    log("Response sent OK")
                except Exception as e:
                    log(f"Output error: {type(e).__name__}: {e}")
                    # Try fallback
                    try:
                        fallback = '{"jsonrpc":"2.0","id":1,"error":{"code":-32603,"message":"Output error"}}\n'
                        sys.stdout.buffer.write(fallback.encode('utf-8'))
                        sys.stdout.buffer.flush()
                    except:
                        pass

        except Exception as e:
            log(f"Loop error: {type(e).__name__}: {e}")

if __name__ == "__main__":
    try:
        main()
    except KeyboardInterrupt:
        log("Interrupted")
        sys.exit(0)
    except Exception as e:
        log(f"FATAL: {type(e).__name__}: {e}")
        try:
            fallback = '{"jsonrpc":"2.0","id":1,"error":{"code":-32603,"message":"Fatal error"}}\n'
            sys.stdout.buffer.write(fallback.encode('utf-8'))
            sys.stdout.buffer.flush()
        except:
            pass
        sys.exit(1)
