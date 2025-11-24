#!/usr/bin/env python3
"""Test the MCP bridge with the new search_content tool"""

import sys
import json

# Simulate MCP protocol messages

# Test 1: tools/list - should include search_content
print("=== Test 1: tools/list ===")
tools_list_request = {
    "jsonrpc": "2.0",
    "id": 1,
    "method": "tools/list"
}

print(f"Simulating stdin: {json.dumps(tools_list_request)}")

# We would pipe this to mcp_bridge.py via stdin in real use
# For now, let's verify the bridge file has the tool definition
import subprocess
result = subprocess.run(
    ["python3", "mcp_bridge.py"],
    input=json.dumps(tools_list_request) + "\n",
    capture_output=True,
    text=True,
    timeout=5
)

if result.returncode == 0:
    response = json.loads(result.stdout.strip())
    tools = response.get("result", {}).get("tools", [])

    # Find search_content tool
    search_content_tool = next((t for t in tools if t["name"] == "mcp_docjl_search_content"), None)

    if search_content_tool:
        print("✅ search_content tool found in tools/list!")
        print(f"   Description: {search_content_tool['description']}")
        print(f"   Required params: {search_content_tool['inputSchema'].get('required', [])}")
    else:
        print("❌ search_content tool NOT found in tools/list")
        print(f"   Available tools: {[t['name'] for t in tools]}")
else:
    print(f"❌ Bridge failed: {result.stderr}")

print()

# Test 2: tools/call - search_content
print("=== Test 2: tools/call search_content ===")
tools_call_request = {
    "jsonrpc": "2.0",
    "id": 2,
    "method": "tools/call",
    "params": {
        "name": "mcp_docjl_search_content",
        "arguments": {
            "document_id": "mk_manual_v1",
            "query": "gázelemző",
            "case_sensitive": False,
            "max_results": 5
        }
    }
}

print(f"Simulating stdin: {json.dumps(tools_call_request)}")

result = subprocess.run(
    ["python3", "mcp_bridge.py"],
    input=json.dumps(tools_call_request) + "\n",
    capture_output=True,
    text=True,
    timeout=5
)

if result.returncode == 0:
    response = json.loads(result.stdout.strip())

    if "error" in response:
        print(f"❌ Error: {response['error']}")
    else:
        # Parse the result
        content = response.get("result", {}).get("content", [])
        if content and len(content) > 0:
            text_content = content[0].get("text", "")
            # The text should contain JSON with search results
            try:
                search_result = json.loads(text_content)
                print(f"✅ Search successful!")
                print(f"   Document: {search_result.get('document_id')}")
                print(f"   Query: {search_result.get('query')}")
                print(f"   Matches: {search_result.get('total_matches')}")
            except:
                print(f"❌ Could not parse search results: {text_content[:100]}")
        else:
            print(f"❌ No content in response")
else:
    print(f"❌ Bridge failed: {result.stderr}")

print("\n=== Bridge test complete ===")
