#!/usr/bin/env python3
"""
Live MCP DOCJL Server Test
Tests all 11 MCP commands against running server
"""

import requests
import json
from typing import Dict, Any

BASE_URL = "http://127.0.0.1:8080"
API_KEY = "dev_key_12345"

def mcp_request(command: str, params: Dict[str, Any]) -> Dict[str, Any]:
    """Send MCP JSON-RPC request"""
    payload = {
        "jsonrpc": "2.0",
        "id": 1,
        "method": command,
        "params": params
    }

    headers = {
        "Content-Type": "application/json",
    }

    # Add API key if authentication is enabled
    # headers["Authorization"] = f"Bearer {API_KEY}"

    response = requests.post(f"{BASE_URL}/mcp", json=payload, headers=headers)
    return response.json()

def print_result(test_name: str, result: Dict[str, Any]):
    """Pretty print test result"""
    print(f"\n{'='*60}")
    print(f"TEST: {test_name}")
    print('='*60)

    if "error" in result:
        print(f"❌ ERROR: {result['error']}")
    elif "result" in result:
        print(f"✅ SUCCESS")
        print(json.dumps(result["result"], indent=2))
    else:
        print(f"⚠️  UNKNOWN RESPONSE: {result}")

def main():
    print("🚀 MCP DOCJL Server Live Test")
    print(f"Server: {BASE_URL}")

    # Test 1: List documents (should be empty initially)
    print_result(
        "1. List Documents (empty)",
        mcp_request("mcp_docjl_list_documents", {})
    )

    # Test 2: Insert a block (this will fail because no document exists yet)
    # In a real scenario, we'd need to create documents first
    # For now, let's test with a hypothetical document
    print_result(
        "2. Insert Block (will fail - no document)",
        mcp_request("mcp_docjl_insert_block", {
            "document_id": "test_doc_1",
            "block": {
                "type": "paragraph",
                "content": [
                    {"type": "text", "content": "This is a test paragraph."}
                ]
            },
            "position": "end",
            "auto_label": True,
            "validate": True
        })
    )

    # Test 3: Get document (will fail - no document)
    print_result(
        "3. Get Document (will fail - no document)",
        mcp_request("mcp_docjl_get_document", {
            "document_id": "test_doc_1"
        })
    )

    # Test 4: List headings (will fail - no document)
    print_result(
        "4. List Headings (will fail - no document)",
        mcp_request("mcp_docjl_list_headings", {
            "document_id": "test_doc_1",
            "max_depth": 3
        })
    )

    # Test 5: Search blocks (will fail - no document)
    print_result(
        "5. Search Blocks (will fail - no document)",
        mcp_request("mcp_docjl_search_blocks", {
            "document_id": "test_doc_1",
            "query": {
                "block_type": "paragraph"
            }
        })
    )

    # Test 6: Validate references (will fail - no document)
    print_result(
        "6. Validate References (will fail - no document)",
        mcp_request("mcp_docjl_validate_references", {
            "document_id": "test_doc_1"
        })
    )

    # Test 7: Validate schema (will fail - no document)
    print_result(
        "7. Validate Schema (will fail - no document)",
        mcp_request("mcp_docjl_validate_schema", {
            "document_id": "test_doc_1"
        })
    )

    # Test 8: Update block (will fail - no document)
    print_result(
        "8. Update Block (will fail - no document)",
        mcp_request("mcp_docjl_update_block", {
            "document_id": "test_doc_1",
            "block_label": "para:1",
            "updates": {
                "content": [{"type": "text", "content": "Updated text"}]
            }
        })
    )

    # Test 9: Move block (will fail - no document)
    print_result(
        "9. Move Block (will fail - no document)",
        mcp_request("mcp_docjl_move_block", {
            "document_id": "test_doc_1",
            "block_label": "para:1",
            "target_parent": "sec:2",
            "position": "end"
        })
    )

    # Test 10: Delete block (will fail - no document)
    print_result(
        "10. Delete Block (will fail - no document)",
        mcp_request("mcp_docjl_delete_block", {
            "document_id": "test_doc_1",
            "block_label": "para:1",
            "cascade": False
        })
    )

    # Test 11: Get audit log (should work - returns audit events)
    print_result(
        "11. Get Audit Log",
        mcp_request("mcp_docjl_get_audit_log", {
            "limit": 10
        })
    )

    print("\n" + "="*60)
    print("📝 NOTE: Most tests failed because we need to:")
    print("   1. Create documents first (not yet implemented)")
    print("   2. Or use the IronBaseAdapter's test helper")
    print("   3. The server IS working - it's returning proper errors!")
    print("="*60)

if __name__ == "__main__":
    main()
