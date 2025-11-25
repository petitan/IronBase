#!/usr/bin/env python3
"""
Check if sec:3 exists and test a simple insert
"""
import sys
sys.path.insert(0, 'examples')
from python_client import MCPDocJLClient
import json

client = MCPDocJLClient()

# Check document
doc = client.get_document('mk_manual_v1')
print(f'Document has {len(doc["docjll"])} blocks')

# Find all labels
labels = [b.get('label') for b in doc['docjll'] if b.get('label')]
print(f'Found {len(labels)} blocks with labels')

# Check for sec:3
if 'sec:3' in labels:
    print('✅ sec:3 EXISTS')
    idx = next(i for i, b in enumerate(doc['docjll']) if b.get('label') == 'sec:3')
    print(f'   Index: {idx}')
    print(f'   Block: {json.dumps(doc["docjll"][idx], ensure_ascii=False)[:200]}')
else:
    print('❌ sec:3 DOES NOT EXIST')
    # Show first 20 section labels
    sec_labels = [l for l in labels if l and l.startswith('sec:')]
    print(f'Section labels found: {sec_labels[:20]}')

# Try to insert a simple block at END position (should work)
print('\n=== Testing END position insert (should work) ===')
try:
    result = client.insert_block(
        document_id='mk_manual_v1',
        block={
            'type': 'paragraph',
            'content': [{'type': 'text', 'content': 'TEST: End insert'}]
        },
        position='end'
    )
    print(f'✅ END insert SUCCESS: {result["block_label"]}')
except Exception as e:
    print(f'❌ END insert FAILED: {e}')
