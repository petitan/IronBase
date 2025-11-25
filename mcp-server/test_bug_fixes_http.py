#!/usr/bin/env python3
"""
HTTP Test for 8 Bug Fixes (Direct HTTP, NOT MCP protocol wrapper)
"""
import sys
sys.path.insert(0, '/home/petitan/MongoLite/venv/lib/python3.12/site-packages')

import requests
import json

BASE_URL = "http://127.0.0.1:8080/mcp"

def send_http_request(method, params):
    """Send direct HTTP request (NOT MCP tools/call wrapper)"""
    payload = {
        "method": method,
        "params": params
    }
    response = requests.post(BASE_URL, json=payload)
    return response.json()

def test_bug_3_position_start():
    """Test Bug #3: Support for 'start' position"""
    print("\n=== Test Bug #3: Position 'start' ===")

    # Insert at start
    result = send_http_request("mcp_docjl_insert_block", {
        "document_id": "test_mcp",
        "block": {
            "type": "paragraph",
            "label": "para:start_test",
            "content": [{"type": "text", "content": "This should be at the start"}]
        },
        "position": "start"
    })

    print(f"Insert result: {json.dumps(result, indent=2)}")

    # Get document to verify
    doc_result = send_http_request("mcp_docjl_get_document", {
        "document_id": "test_mcp"
    })

    doc_data = doc_result.get('result', {})
    first_block_label = doc_data['document']['docjll'][0].get('label', 'NONE')
    print(f"First block label: {first_block_label}")

    if first_block_label == "para:start_test":
        print("✅ Bug #3 PASSED: 'start' position works correctly")
        return True
    else:
        print("❌ Bug #3 FAILED: Expected para:start_test at position 0")
        return False

def test_bug_4_level_filtering():
    """Test Bug #4: Level filtering in search"""
    print("\n=== Test Bug #4: Level filtering ===")

    # Search for level 1 headings
    result = send_http_request("mcp_docjl_search_blocks", {
        "document_id": "test_mcp",
        "query": {
            "type": "heading",
            "level": 1
        }
    })

    results = result.get('result', {}).get('results', [])
    print(f"Found {len(results)} level 1 headings")

    # Verify all results are level 1
    all_level_1 = all(
        r.get('block', {}).get('level') == 1
        for r in results
        if r.get('block', {}).get('type') == 'heading'
    )

    if all_level_1 and len(results) >= 2:
        print("✅ Bug #4 PASSED: Level filtering works correctly")
        return True
    else:
        print(f"❌ Bug #4 FAILED: Not all results are level 1 or count is wrong")
        print(f"Results: {json.dumps(results, indent=2)}")
        return False

def test_bug_6_duplicate_label_validation():
    """Test Bug #6: Duplicate label validation"""
    print("\n=== Test Bug #6: Duplicate label validation ===")

    # Try to insert a block with existing label
    result = send_http_request("mcp_docjl_insert_block", {
        "document_id": "test_mcp",
        "block": {
            "type": "paragraph",
            "label": "sec:1",  # This already exists!
            "content": [{"type": "text", "content": "Duplicate label test"}]
        },
        "position": "end"
    })

    print(f"Result: {json.dumps(result, indent=2)}")

    # Should get an error about duplicate label
    if "error" in result:
        error_msg = result["error"]["message"]
        if "Duplicate" in error_msg or "duplicate" in error_msg:
            print("✅ Bug #6 PASSED: Duplicate label validation works")
            return True

    print(f"❌ Bug #6 FAILED: Expected duplicate label error")
    return False

def test_bug_7_list_headings():
    """Test Bug #7: List headings without duplicates"""
    print("\n=== Test Bug #7: List headings (no duplicates) ===")

    result = send_http_request("mcp_docjl_list_headings", {
        "document_id": "test_mcp"
    })

    outline = result.get('result', {}).get('outline', [])

    # Count total headings (including nested)
    def count_headings(items):
        count = len(items)
        for item in items:
            count += count_headings(item.get('children', []))
        return count

    total_headings = count_headings(outline)
    print(f"Total headings in outline: {total_headings}")

    # We expect: sec:1 (with child sec:1.1) and sec:2 = 3 headings
    # Before the fix, sec:1.1 would appear twice
    if total_headings == 3:
        print("✅ Bug #7 PASSED: No duplicate headings in outline")
        return True
    else:
        print(f"❌ Bug #7 FAILED: Expected 3 headings, got {total_headings}")
        print(f"Outline: {json.dumps(outline, indent=2)}")
        return False

def test_bug_8_alphanumeric_labels():
    """Test Bug #8: Alphanumeric label support"""
    print("\n=== Test Bug #8: Alphanumeric labels ===")

    # Try to insert a block with alphanumeric label
    result = send_http_request("mcp_docjl_insert_block", {
        "document_id": "test_mcp",
        "block": {
            "type": "paragraph",
            "label": "para:test_alpha",  # Alphanumeric!
            "content": [{"type": "text", "content": "Alphanumeric label test"}]
        },
        "position": "end"
    })

    print(f"Result: {json.dumps(result, indent=2)}")

    success = result.get('result', {}).get('success', False)
    block_label = result.get('result', {}).get('block_label', '')

    if success and block_label == "para:test_alpha":
        print("✅ Bug #8 PASSED: Alphanumeric labels are accepted")
        return True
    else:
        print(f"❌ Bug #8 FAILED: Expected success with para:test_alpha")
        return False

def main():
    print("=" * 60)
    print("HTTP Bug Fix Tests (Direct HTTP Format)")
    print("=" * 60)

    results = {
        "Bug #3 (position: start)": test_bug_3_position_start(),
        "Bug #4 (level filtering)": test_bug_4_level_filtering(),
        "Bug #6 (duplicate labels)": test_bug_6_duplicate_label_validation(),
        "Bug #7 (list headings)": test_bug_7_list_headings(),
        "Bug #8 (alphanumeric labels)": test_bug_8_alphanumeric_labels(),
    }

    print("\n" + "=" * 60)
    print("Summary")
    print("=" * 60)

    for test_name, passed in results.items():
        status = "✅ PASSED" if passed else "❌ FAILED"
        print(f"{test_name}: {status}")

    total = len(results)
    passed = sum(results.values())
    print(f"\nTotal: {passed}/{total} tests passed")

    return 0 if passed == total else 1

if __name__ == "__main__":
    sys.exit(main())
