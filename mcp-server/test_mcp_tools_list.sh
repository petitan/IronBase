#!/bin/bash
# Test MCP tools/list handler

set -e

echo "============================================================"
echo "Test: MCP tools/list Handler"
echo "============================================================"

# Test tools/list
echo ""
echo "Test: tools/list (9 DOCJL tools)"
RESPONSE=$(curl -s -X POST http://127.0.0.1:8080/mcp \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc": "2.0",
    "id": 1,
    "method": "tools/list",
    "params": {}
  }')

echo "$RESPONSE" | python3 -m json.tool

# Validate response
echo ""
echo "Validating response..."

# Check jsonrpc field
if echo "$RESPONSE" | grep -q '"jsonrpc": "2.0"'; then
  echo "✅ jsonrpc field correct"
else
  echo "❌ FAILED: Missing or incorrect jsonrpc field"
  exit 1
fi

# Check tools array
if echo "$RESPONSE" | grep -q '"tools"'; then
  echo "✅ tools field present"
else
  echo "❌ FAILED: Missing tools field"
  exit 1
fi

# Count tools (should be 9)
TOOL_COUNT=$(echo "$RESPONSE" | python3 -c "import sys, json; d=json.load(sys.stdin); print(len(d['result']['tools']))")
if [ "$TOOL_COUNT" -eq 9 ]; then
  echo "✅ Tool count correct: 9 tools"
else
  echo "❌ FAILED: Expected 9 tools, got $TOOL_COUNT"
  exit 1
fi

# Check specific tools exist
EXPECTED_TOOLS=(
  "mcp_docjl_create_document"
  "mcp_docjl_list_documents"
  "mcp_docjl_get_document"
  "mcp_docjl_list_headings"
  "mcp_docjl_search_blocks"
  "mcp_docjl_search_content"
  "mcp_docjl_insert_block"
  "mcp_docjl_update_block"
  "mcp_docjl_delete_block"
)

for TOOL in "${EXPECTED_TOOLS[@]}"; do
  if echo "$RESPONSE" | grep -q "\"name\": \"$TOOL\""; then
    echo "✅ Tool found: $TOOL"
  else
    echo "❌ FAILED: Missing tool: $TOOL"
    exit 1
  fi
done

# Check that each tool has description and inputSchema
echo ""
echo "Validating tool schemas..."
TOOLS_WITH_SCHEMA=$(echo "$RESPONSE" | python3 -c "
import sys, json
d = json.load(sys.stdin)
tools = d['result']['tools']
valid = 0
for tool in tools:
    if 'name' in tool and 'description' in tool and 'inputSchema' in tool:
        valid += 1
print(valid)
")

if [ "$TOOLS_WITH_SCHEMA" -eq 9 ]; then
  echo "✅ All 9 tools have name, description, and inputSchema"
else
  echo "❌ FAILED: Only $TOOLS_WITH_SCHEMA/9 tools have complete schema"
  exit 1
fi

echo ""
echo "============================================================"
echo "✅ ALL TESTS PASSED - tools/list handler working correctly"
echo "============================================================"
