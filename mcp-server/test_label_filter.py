#!/usr/bin/env python3
"""
Test the label filter bug fix in search_blocks
"""
import sys
sys.path.insert(0, 'examples')
from python_client import MCPDocJLClient

client = MCPDocJLClient()

print("=" * 60)
print("TEST: Label Filter in search_blocks")
print("=" * 60)

# Test 1: Exact label match (this was broken before)
print("\n📌 Test 1: Exact label match - label='sec:14'")
print("-" * 60)
try:
    result = client.search_blocks(
        document_id="mk_manual_v1",
        label="sec:14"
    )
    print(f"✅ SUCCESS - Found {len(result)} blocks")

    if len(result) == 1:
        print(f"   ✅ CORRECT: Expected 1 block, got 1")
        print(f"   Label: {result[0]['label']}")
        print(f"   Block type: {result[0]['block']['type']}")
    else:
        print(f"   ❌ INCORRECT: Expected 1 block, but got {len(result)}")
        if len(result) > 0:
            print(f"   Labels found: {[r['label'] for r in result[:5]]}")
except Exception as e:
    print(f"❌ FAILED: {e}")

# Test 2: Label prefix match
print("\n📌 Test 2: Label prefix match - label_prefix='sec:'")
print("-" * 60)
try:
    result = client.search_blocks(
        document_id="mk_manual_v1",
        label_prefix="sec:"
    )
    print(f"✅ SUCCESS - Found {len(result)} blocks with 'sec:' prefix")

    if len(result) > 1:
        print(f"   Sample labels: {[r['label'] for r in result[:5]]}")
    else:
        print(f"   ⚠️ WARNING: Expected multiple blocks, got {len(result)}")
except Exception as e:
    print(f"❌ FAILED: {e}")

# Test 3: Exact label match for paragraph
print("\n📌 Test 3: Exact label match - label='para:1'")
print("-" * 60)
try:
    result = client.search_blocks(
        document_id="mk_manual_v1",
        label="para:1"
    )
    print(f"✅ SUCCESS - Found {len(result)} blocks")

    if len(result) == 1:
        print(f"   ✅ CORRECT: Expected 1 block, got 1")
        print(f"   Label: {result[0]['label']}")
        print(f"   Block type: {result[0]['block']['type']}")
    else:
        print(f"   ❌ INCORRECT: Expected 1 block, but got {len(result)}")
except Exception as e:
    print(f"❌ FAILED: {e}")

# Test 4: Block type filter (should still work)
print("\n📌 Test 4: Block type filter - type='heading'")
print("-" * 60)
try:
    result = client.search_blocks(
        document_id="mk_manual_v1",
        block_type="heading"
    )
    print(f"✅ SUCCESS - Found {len(result)} heading blocks")

    if len(result) > 0:
        print(f"   Sample labels: {[r['label'] for r in result[:5]]}")
except Exception as e:
    print(f"❌ FAILED: {e}")

# Test 5: Combined filters (label + type)
print("\n📌 Test 5: Combined filters - label='sec:14' + type='heading'")
print("-" * 60)
try:
    result = client.search_blocks(
        document_id="mk_manual_v1",
        label="sec:14",
        block_type="heading"
    )
    print(f"✅ SUCCESS - Found {len(result)} blocks")

    if len(result) == 1:
        print(f"   ✅ CORRECT: Expected 1 block, got 1")
        print(f"   Label: {result[0]['label']}")
        print(f"   Block type: {result[0]['block']['type']}")
    elif len(result) == 0:
        print(f"   ⚠️ NOTE: sec:14 might not be a heading (got 0 results)")
    else:
        print(f"   ❌ INCORRECT: Expected 1 block, but got {len(result)}")
except Exception as e:
    print(f"❌ FAILED: {e}")

print("\n" + "=" * 60)
print("TEST SUITE COMPLETED")
print("=" * 60)
