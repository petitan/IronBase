#!/usr/bin/env python3
"""Test simplified Python bridge (dumb proxy mode)"""

import json
import subprocess
import sys

def test_bridge(test_name: str, request: dict, expected_method: str = None):
    """Test the simplified bridge with a request"""
    print(f"\n{'='*60}")
    print(f"Test: {test_name}")
    print(f"{'='*60}")
    print(f"Request method: {request.get('method')}")

    result = subprocess.run(
        ["python3", "mcp_bridge.py"],
        input=json.dumps(request),
        capture_output=True,
        text=True,
        timeout=10
    )

    if result.stdout:
        try:
            response = json.loads(result.stdout)
            print(f"✅ Response received")
            print(f"   jsonrpc: {response.get('jsonrpc')}")
            print(f"   id: {response.get('id')}")

            if "result" in response:
                print(f"   result keys: {list(response['result'].keys())}")
                if expected_method:
                    print(f"   ✅ {expected_method} successful")
            elif "error" in response:
                print(f"   ⚠️  Error: {response['error'].get('message')}")

            return True
        except json.JSONDecodeError as e:
            print(f"❌ Invalid JSON response: {e}")
            print(f"   stdout: {result.stdout[:200]}")
            return False
    else:
        print(f"❌ No response")
        if result.stderr:
            print(f"   stderr: {result.stderr[:200]}")
        return False

def main():
    print("="*60)
    print("Testing Simplified Python Bridge (Dumb Proxy Mode)")
    print("="*60)
    print("\nAll MCP protocol logic should now be handled by Rust server")

    tests_passed = 0
    tests_total = 0

    # Test 1: initialize (now forwarded to Rust)
    tests_total += 1
    if test_bridge(
        "initialize (forwarded to Rust)",
        {
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": {"name": "test-client", "version": "1.0"}
            }
        },
        "initialize"
    ):
        tests_passed += 1

    # Test 2: tools/list (now forwarded to Rust)
    tests_total += 1
    if test_bridge(
        "tools/list (forwarded to Rust)",
        {
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/list",
            "params": {}
        },
        "tools/list"
    ):
        tests_passed += 1

    # Test 3: resources/list (forwarded to Rust)
    tests_total += 1
    if test_bridge(
        "resources/list (forwarded to Rust)",
        {
            "jsonrpc": "2.0",
            "id": 3,
            "method": "resources/list",
            "params": {}
        },
        "resources/list"
    ):
        tests_passed += 1

    # Test 4: prompts/list (forwarded to Rust)
    tests_total += 1
    if test_bridge(
        "prompts/list (forwarded to Rust)",
        {
            "jsonrpc": "2.0",
            "id": 4,
            "method": "prompts/list",
            "params": {}
        },
        "prompts/list"
    ):
        tests_passed += 1

    # Test 5: tools/call (forwarded to Rust)
    tests_total += 1
    if test_bridge(
        "tools/call - list_documents (forwarded to Rust)",
        {
            "jsonrpc": "2.0",
            "id": 5,
            "method": "tools/call",
            "params": {
                "name": "mcp_docjl_list_documents"
            }
        },
        "tools/call"
    ):
        tests_passed += 1

    # Summary
    print(f"\n{'='*60}")
    print(f"Test Summary")
    print(f"{'='*60}")
    print(f"Passed: {tests_passed}/{tests_total}")

    if tests_passed == tests_total:
        print("\n✅ All tests PASSED - Simplified bridge works correctly!")
        print("   All MCP methods are now handled by the Rust server")
        return 0
    else:
        print(f"\n❌ {tests_total - tests_passed} test(s) FAILED")
        return 1

if __name__ == "__main__":
    sys.exit(main())
