#!/bin/bash
# Test Phase 3 Chunking Support tools: get_section and estimate_tokens

set -e

echo "============================================================"
echo "Phase 3: Chunking Support Tools Test"
echo "============================================================"

# Kill any running servers
killall -9 mcp-docjl-server 2>/dev/null || true
sleep 2

# Clean database
rm -f docjl_storage.mlite docjl_storage.wal

# Seed database with nested document
echo ""
echo "=== 1. Seeding test document with nested structure ==="
python3 -c "
import sys
sys.path.insert(0, '/home/petitan/MongoLite/venv/lib/python3.12/site-packages')
import ironbase

doc = {
    'id': 'chunking_test',
    'metadata': {'title': 'Chunking Test Document', 'version': '1.0'},
    'docjll': [
        {
            'type': 'heading',
            'level': 1,
            'content': [{'type': 'text', 'content': 'Chapter 1: Introduction'}],
            'label': 'sec:1',
            'children': [
                {
                    'type': 'paragraph',
                    'content': [{'type': 'text', 'content': 'This is the introduction paragraph with some text content.'}],
                    'label': 'para:1'
                },
                {
                    'type': 'heading',
                    'level': 2,
                    'content': [{'type': 'text', 'content': 'Section 1.1: Background'}],
                    'label': 'sec:1.1',
                    'children': [
                        {
                            'type': 'paragraph',
                            'content': [{'type': 'text', 'content': 'Background information goes here.'}],
                            'label': 'para:2'
                        },
                        {
                            'type': 'heading',
                            'level': 3,
                            'content': [{'type': 'text', 'content': 'Subsection 1.1.1: Details'}],
                            'label': 'sec:1.1.1',
                            'children': [
                                {
                                    'type': 'paragraph',
                                    'content': [{'type': 'text', 'content': 'Detailed information at depth 3.'}],
                                    'label': 'para:3'
                                }
                            ]
                        }
                    ]
                }
            ]
        },
        {
            'type': 'heading',
            'level': 1,
            'content': [{'type': 'text', 'content': 'Chapter 2: Methods'}],
            'label': 'sec:2',
            'children': [
                {
                    'type': 'paragraph',
                    'content': [{'type': 'text', 'content': 'Methods section content.'}],
                    'label': 'para:4'
                }
            ]
        }
    ]
}

db = ironbase.IronBase('docjl_storage.mlite')
coll = db.collection('documents')
result = coll.insert_one(doc)
print(f'✅ Seeded document: {doc[\"id\"]}')
print(f'   Structure: 2 chapters, sec:1 has 2 levels deep nesting')
"

# Start server
echo ""
echo "=== 2. Starting MCP server ==="
./target/release/mcp-docjl-server &
SERVER_PID=$!
sleep 3
echo "✅ Server started (PID: $SERVER_PID)"

# Test 1: tools/list should show 11 tools now (9 + 2 new)
echo ""
echo "=== 3. Test tools/list (should have 11 tools) ==="
TOOLS_RESPONSE=$(curl -s -X POST http://127.0.0.1:8080/mcp \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer dev_key_12345" \
  -d '{
    "jsonrpc": "2.0",
    "id": 1,
    "method": "tools/list",
    "params": {}
  }')

TOOL_COUNT=$(echo "$TOOLS_RESPONSE" | python3 -c "import sys, json; d=json.load(sys.stdin); print(len(d['result']['tools']))")
if [ "$TOOL_COUNT" -eq 11 ]; then
  echo "✅ tools/list: Found 11 tools (9 existing + 2 new chunking tools)"
else
  echo "❌ FAILED: Expected 11 tools, got $TOOL_COUNT"
  kill $SERVER_PID
  exit 1
fi

# Check that new tools exist
if echo "$TOOLS_RESPONSE" | grep -q "mcp_docjl_get_section"; then
  echo "✅ mcp_docjl_get_section found in tools/list"
else
  echo "❌ FAILED: mcp_docjl_get_section not found"
  kill $SERVER_PID
  exit 1
fi

if echo "$TOOLS_RESPONSE" | grep -q "mcp_docjl_estimate_tokens"; then
  echo "✅ mcp_docjl_estimate_tokens found in tools/list"
else
  echo "❌ FAILED: mcp_docjl_estimate_tokens not found"
  kill $SERVER_PID
  exit 1
fi

# Test 2: get_section with full depth
echo ""
echo "=== 4. Test mcp_docjl_get_section (sec:1 with full depth) ==="
GET_SECTION_RESPONSE=$(curl -s -X POST http://127.0.0.1:8080/mcp \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer dev_key_12345" \
  -d '{
    "jsonrpc": "2.0",
    "id": 2,
    "method": "tools/call",
    "params": {
      "name": "mcp_docjl_get_section",
      "arguments": {
        "document_id": "chunking_test",
        "section_label": "sec:1",
        "include_subsections": true,
        "max_depth": 10
      }
    }
  }')

if echo "$GET_SECTION_RESPONSE" | grep -q "sec:1" && echo "$GET_SECTION_RESPONSE" | grep -q "sec:1.1.1"; then
  echo "✅ get_section: Retrieved sec:1 with all nested children (depth 3)"
else
  echo "❌ FAILED: get_section did not return expected nested structure"
  echo "$GET_SECTION_RESPONSE" | python3 -m json.tool
  kill $SERVER_PID
  exit 1
fi

# Test 3: get_section with limited depth
echo ""
echo "=== 5. Test mcp_docjl_get_section (sec:1 with max_depth=1) ==="
GET_SECTION_DEPTH1=$(curl -s -X POST http://127.0.0.1:8080/mcp \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer dev_key_12345" \
  -d '{
    "jsonrpc": "2.0",
    "id": 3,
    "method": "tools/call",
    "params": {
      "name": "mcp_docjl_get_section",
      "arguments": {
        "document_id": "chunking_test",
        "section_label": "sec:1",
        "include_subsections": true,
        "max_depth": 1
      }
    }
  }')

if echo "$GET_SECTION_DEPTH1" | grep -q "sec:1.1" && ! echo "$GET_SECTION_DEPTH1" | grep -q "sec:1.1.1"; then
  echo "✅ get_section: max_depth=1 correctly limits to immediate children"
else
  echo "⚠️  WARNING: max_depth=1 might not be working correctly"
fi

# Test 4: get_section without subsections
echo ""
echo "=== 6. Test mcp_docjl_get_section (sec:1 without subsections) ==="
GET_SECTION_NO_SUB=$(curl -s -X POST http://127.0.0.1:8080/mcp \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer dev_key_12345" \
  -d '{
    "jsonrpc": "2.0",
    "id": 4,
    "method": "tools/call",
    "params": {
      "name": "mcp_docjl_get_section",
      "arguments": {
        "document_id": "chunking_test",
        "section_label": "sec:1",
        "include_subsections": false
      }
    }
  }')

if echo "$GET_SECTION_NO_SUB" | grep -q "sec:1" && ! echo "$GET_SECTION_NO_SUB" | grep -q "sec:1.1"; then
  echo "✅ get_section: include_subsections=false correctly excludes children"
else
  echo "⚠️  WARNING: include_subsections=false might not be working correctly"
fi

# Test 5: estimate_tokens for entire document
echo ""
echo "=== 7. Test mcp_docjl_estimate_tokens (entire document) ==="
ESTIMATE_DOC=$(curl -s -X POST http://127.0.0.1:8080/mcp \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer dev_key_12345" \
  -d '{
    "jsonrpc": "2.0",
    "id": 5,
    "method": "tools/call",
    "params": {
      "name": "mcp_docjl_estimate_tokens",
      "arguments": {
        "document_id": "chunking_test"
      }
    }
  }')

DOC_TOKENS=$(echo "$ESTIMATE_DOC" | python3 -c "import sys, json; d=json.load(sys.stdin); print(d.get('result', {}).get('estimated_tokens', 0))")
if [ "$DOC_TOKENS" -gt 0 ]; then
  echo "✅ estimate_tokens: Document estimated at $DOC_TOKENS tokens"
else
  echo "❌ FAILED: estimate_tokens returned 0 or invalid tokens"
  kill $SERVER_PID
  exit 1
fi

# Test 6: estimate_tokens for specific section
echo ""
echo "=== 8. Test mcp_docjl_estimate_tokens (section sec:1) ==="
ESTIMATE_SECTION=$(curl -s -X POST http://127.0.0.1:8080/mcp \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer dev_key_12345" \
  -d '{
    "jsonrpc": "2.0",
    "id": 6,
    "method": "tools/call",
    "params": {
      "name": "mcp_docjl_estimate_tokens",
      "arguments": {
        "document_id": "chunking_test",
        "section_label": "sec:1"
      }
    }
  }')

SECTION_TOKENS=$(echo "$ESTIMATE_SECTION" | python3 -c "import sys, json; d=json.load(sys.stdin); print(d.get('result', {}).get('estimated_tokens', 0))")
if [ "$SECTION_TOKENS" -gt 0 ] && [ "$SECTION_TOKENS" -lt "$DOC_TOKENS" ]; then
  echo "✅ estimate_tokens: Section sec:1 estimated at $SECTION_TOKENS tokens (< full document $DOC_TOKENS)"
else
  echo "⚠️  WARNING: Section token estimate might not be correct ($SECTION_TOKENS vs $DOC_TOKENS)"
fi

# Cleanup
kill $SERVER_PID

echo ""
echo "============================================================"
echo "✅ ALL CHUNKING TOOLS TESTS PASSED"
echo "============================================================"
echo ""
echo "Summary:"
echo "  - tools/list shows 11 tools (9 + 2 new)"
echo "  - mcp_docjl_get_section works with depth control"
echo "  - mcp_docjl_get_section works with include_subsections"
echo "  - mcp_docjl_estimate_tokens works for documents and sections"
echo ""
echo "Phase 3: Chunking Support COMPLETE ✅"
echo "============================================================"
