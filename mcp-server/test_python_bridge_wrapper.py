#!/usr/bin/env python3
"""Test Python bridge MCP wrapper support"""

import json
import subprocess
import sys
import time

def main():
    print("=== Testing Python Bridge MCP Wrapper Support ===\n")

    # Simulate Claude Desktop sending tools/list
    print("Test 1: tools/list")
    request = {
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/list",
        "params": {}
    }

    result = subprocess.run(
        ["python3", "mcp_bridge.py"],
        input=json.dumps(request),
        capture_output=True,
        text=True
    )

    if result.stdout:
        response = json.loads(result.stdout)
        print(f"Response: {json.dumps(response, indent=2)}\n")
        assert response.get("jsonrpc") == "2.0", "Missing jsonrpc field"
        assert response.get("id") == 1, "Missing id field"
        print("✅ Test 1 PASSED\n")
    else:
        print(f"❌ Test 1 FAILED: {result.stderr}\n")
        return 1

    # Simulate Claude Desktop sending tools/call
    print("Test 2: tools/call (list documents)")
    request = {
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/call",
        "params": {
            "name": "mcp_docjl_list_documents"
        }
    }

    result = subprocess.run(
        ["python3", "mcp_bridge.py"],
        input=json.dumps(request),
        capture_output=True,
        text=True,
        timeout=10
    )

    if result.stdout:
        response = json.loads(result.stdout)
        print(f"Response: {json.dumps(response, indent=2)}\n")
        assert response.get("jsonrpc") == "2.0", "Missing jsonrpc field"
        assert response.get("id") == 2, "Missing id field"
        assert "result" in response or "error" in response, "Missing result/error"
        print("✅ Test 2 PASSED\n")
    else:
        print(f"❌ Test 2 FAILED: {result.stderr}\n")
        return 1

    print("=== All Python Bridge tests PASSED ===")
    return 0

if __name__ == "__main__":
    sys.exit(main())
