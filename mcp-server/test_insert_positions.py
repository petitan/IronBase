#!/usr/bin/env python3
"""
Test insert positions: Before, After, Inside
"""
import sys
sys.path.insert(0, 'examples')
from python_client import MCPDocJLClient

client = MCPDocJLClient()

print("=" * 70)
print("TEST: Insert Block Positions (Before/After/Inside)")
print("=" * 70)

# Get initial state
doc = client.get_document('mk_manual_v1')
initial_count = len(doc.get('docjll', []))
print(f"\nInitial document: {initial_count} blocks")

# Find a suitable test location (use sec:3 as anchor)
test_blocks = [b for b in doc['docjll'] if b.get('label') == 'sec:3']
if not test_blocks:
    print("❌ ERROR: Could not find sec:3 for testing")
    sys.exit(1)

anchor_index = next(i for i, b in enumerate(doc['docjll']) if b.get('label') == 'sec:3')
print(f"Found sec:3 at index {anchor_index}")

# Test 1: Insert BEFORE
print("\n📌 Test 1: Insert BEFORE sec:3")
print("-" * 70)
try:
    result = client.insert_block(
        document_id="mk_manual_v1",
        block={
            "type": "paragraph",
            "content": [{"type": "text", "content": "TEST: Inserted BEFORE sec:3"}]
        },
        anchor_label="sec:3",
        position="before"
    )
    print(f"✅ SUCCESS: {result['block_label']}")

    # Verify position
    doc_after = client.get_document('mk_manual_v1')
    new_index = next(i for i, b in enumerate(doc_after['docjll'])
                     if b.get('label') == result['block_label'])
    sec3_index = next(i for i, b in enumerate(doc_after['docjll'])
                      if b.get('label') == 'sec:3')

    if new_index == sec3_index - 1:
        print(f"   ✅ CORRECT POSITION: new block at index {new_index}, sec:3 at {sec3_index}")
    else:
        print(f"   ❌ WRONG POSITION: new block at {new_index}, sec:3 at {sec3_index}")

except Exception as e:
    print(f"❌ FAILED: {e}")

# Test 2: Insert AFTER
print("\n📌 Test 2: Insert AFTER sec:3")
print("-" * 70)
try:
    result = client.insert_block(
        document_id="mk_manual_v1",
        block={
            "type": "paragraph",
            "content": [{"type": "text", "content": "TEST: Inserted AFTER sec:3"}]
        },
        anchor_label="sec:3",
        position="after"
    )
    print(f"✅ SUCCESS: {result['block_label']}")

    # Verify position
    doc_after = client.get_document('mk_manual_v1')
    new_index = next(i for i, b in enumerate(doc_after['docjll'])
                     if b.get('label') == result['block_label'])
    sec3_index = next(i for i, b in enumerate(doc_after['docjll'])
                      if b.get('label') == 'sec:3')

    if new_index == sec3_index + 1:
        print(f"   ✅ CORRECT POSITION: new block at index {new_index}, sec:3 at {sec3_index}")
    else:
        print(f"   ❌ WRONG POSITION: new block at {new_index}, sec:3 at {sec3_index}")

except Exception as e:
    print(f"❌ FAILED: {e}")

# Test 3: Insert INSIDE (should be same as AFTER for flat list)
print("\n📌 Test 3: Insert INSIDE sec:3 (parent_label)")
print("-" * 70)
try:
    result = client.insert_block(
        document_id="mk_manual_v1",
        block={
            "type": "paragraph",
            "content": [{"type": "text", "content": "TEST: Inserted INSIDE sec:3"}]
        },
        parent_label="sec:3",
        position="inside"
    )
    print(f"✅ SUCCESS: {result['block_label']}")

    # Verify position (should be right after parent)
    doc_after = client.get_document('mk_manual_v1')
    new_index = next(i for i, b in enumerate(doc_after['docjll'])
                     if b.get('label') == result['block_label'])
    sec3_index = next(i for i, b in enumerate(doc_after['docjll'])
                      if b.get('label') == 'sec:3')

    if new_index == sec3_index + 1:
        print(f"   ✅ CORRECT POSITION: new block at index {new_index}, sec:3 at {sec3_index}")
    else:
        print(f"   ❌ WRONG POSITION: new block at {new_index}, sec:3 at {sec3_index}")

except Exception as e:
    print(f"❌ FAILED: {e}")

# Test 4: Error handling - Before without anchor_label
print("\n📌 Test 4: Error handling - Before without anchor_label")
print("-" * 70)
try:
    result = client.insert_block(
        document_id="mk_manual_v1",
        block={
            "type": "paragraph",
            "content": [{"type": "text", "content": "This should fail"}]
        },
        position="before"  # Missing anchor_label!
    )
    print(f"❌ SHOULD HAVE FAILED but got: {result}")
except Exception as e:
    if "anchor_label" in str(e).lower():
        print(f"✅ CORRECT ERROR: {e}")
    else:
        print(f"❌ WRONG ERROR: {e}")

# Final count
doc_final = client.get_document('mk_manual_v1')
final_count = len(doc_final.get('docjll', []))
print(f"\n📊 Final document: {final_count} blocks (was {initial_count})")
print(f"   Added {final_count - initial_count} blocks")

print("\n" + "=" * 70)
print("TEST SUITE COMPLETED")
print("=" * 70)
