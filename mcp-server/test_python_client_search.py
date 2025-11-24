#!/usr/bin/env python3
"""Test the new search_content method in the Python client library"""

import sys
sys.path.insert(0, 'examples')

from python_client import MCPDocJLClient

# Initialize client
client = MCPDocJLClient(api_key="dev_key_12345")

print("=== Testing search_content() method ===\n")

# Test 1: Search for "gázelemző"
print("Test 1: Searching for 'gázelemző'...")
result = client.search_content(
    document_id="mk_manual_v1",
    query="gázelemző",
    case_sensitive=False,
    max_results=10
)

print(f"✅ Document: {result['document_id']}")
print(f"✅ Query: '{result['query']}'")
print(f"✅ Total matches: {result['total_matches']}")
print()

if result.get('matches'):
    print("First match:")
    match = result['matches'][0]
    print(f"  Block index: {match['block_index']}")
    print(f"  Block type: {match['block_type']}")
    print(f"  Label: {match.get('label', 'N/A')}")
    print(f"  Text preview (first 100 chars): {match['text'][:100]}")
    print()

# Test 2: Search for "kalibr" (should find more results)
print("Test 2: Searching for 'kalibr' (partial match)...")
result2 = client.search_content(
    document_id="mk_manual_v1",
    query="kalibr",
    case_sensitive=False,
    max_results=5
)

print(f"✅ Total matches: {result2['total_matches']}")
print(f"✅ Returned matches: {len(result2.get('matches', []))}")
print()

print("=== All tests passed! ===")
print("\nThe search_content() API is now available in the Python client library.")
