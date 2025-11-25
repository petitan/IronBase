#!/bin/bash
# Test MCP resources/list and resources/read handlers

set -e

echo "============================================================"
echo "Test: MCP resources/* Handlers"
echo "============================================================"

# Test 1: resources/list (empty database)
echo ""
echo "Test 1: resources/list (should be empty or have existing docs)"
RESPONSE=$(curl -s -X POST http://127.0.0.1:8080/mcp \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc": "2.0",
    "id": 1,
    "method": "resources/list",
    "params": {}
  }')

echo "$RESPONSE" | python3 -m json.tool

# Validate response structure
if echo "$RESPONSE" | grep -q '"resources"'; then
  echo "✅ resources field present"
else
  echo "❌ FAILED: Missing resources field"
  exit 1
fi

RESOURCE_COUNT=$(echo "$RESPONSE" | python3 -c "import sys, json; d=json.load(sys.stdin); print(len(d['result']['resources']))")
echo "   Found $RESOURCE_COUNT resource(s)"

# Test 2: Create a test document
echo ""
echo "Test 2: Creating test document..."
CREATE_RESPONSE=$(curl -s -X POST http://127.0.0.1:8080/mcp \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc": "2.0",
    "id": 2,
    "method": "tools/call",
    "params": {
      "name": "mcp_docjl_create_document",
      "arguments": {
        "document": {
          "id": "test_resource_doc",
          "metadata": {"title": "Resource Test Doc", "version": "1.0"},
          "docjll": [
            {
              "type": "heading",
              "level": 1,
              "content": [{"type": "text", "content": "Test Heading"}],
              "label": "sec:1",
              "children": []
            }
          ]
        }
      }
    }
  }')

if echo "$CREATE_RESPONSE" | grep -q '"success": true'; then
  echo "✅ Document created successfully"
else
  echo "❌ FAILED: Could not create document"
  exit 1
fi

# Test 3: resources/list (should have the new document)
echo ""
echo "Test 3: resources/list (should include new document)"
RESPONSE=$(curl -s -X POST http://127.0.0.1:8080/mcp \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc": "2.0",
    "id": 3,
    "method": "resources/list",
    "params": {}
  }')

if echo "$RESPONSE" | grep -q '"uri": "docjl://document/test_resource_doc"'; then
  echo "✅ Resource found with correct URI format"
else
  echo "❌ FAILED: Resource not found or incorrect URI"
  exit 1
fi

# Check resource has all required fields
HAS_ALL_FIELDS=$(echo "$RESPONSE" | python3 -c "
import sys, json
d = json.load(sys.stdin)
resources = d['result']['resources']
for r in resources:
    if r.get('uri', '').endswith('test_resource_doc'):
        if all(k in r for k in ['uri', 'name', 'description', 'mimeType']):
            print('yes')
            break
")

if [ "$HAS_ALL_FIELDS" = "yes" ]; then
  echo "✅ Resource has all required fields (uri, name, description, mimeType)"
else
  echo "❌ FAILED: Resource missing required fields"
  exit 1
fi

# Test 4: resources/read (existing document)
echo ""
echo "Test 4: resources/read (existing document)"
READ_RESPONSE=$(curl -s -X POST http://127.0.0.1:8080/mcp \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc": "2.0",
    "id": 4,
    "method": "resources/read",
    "params": {
      "uri": "docjl://document/test_resource_doc"
    }
  }')

echo "$READ_RESPONSE" | python3 -m json.tool | head -30

# Validate response
if echo "$READ_RESPONSE" | grep -q '"contents"'; then
  echo "✅ contents field present"
else
  echo "❌ FAILED: Missing contents field"
  exit 1
fi

if echo "$READ_RESPONSE" | grep -q '"uri": "docjl://document/test_resource_doc"'; then
  echo "✅ URI echoed correctly in response"
else
  echo "❌ FAILED: URI not echoed in response"
  exit 1
fi

if echo "$READ_RESPONSE" | grep -q '"mimeType": "application/json"'; then
  echo "✅ mimeType correct (application/json)"
else
  echo "❌ FAILED: Incorrect mimeType"
  exit 1
fi

if echo "$READ_RESPONSE" | grep -q '"text"'; then
  echo "✅ text field present (document JSON)"
else
  echo "❌ FAILED: Missing text field"
  exit 1
fi

# Test 5: resources/read (non-existent document)
echo ""
echo "Test 5: resources/read (non-existent document - should fail)"
ERROR_RESPONSE=$(curl -s -X POST http://127.0.0.1:8080/mcp \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc": "2.0",
    "id": 5,
    "method": "resources/read",
    "params": {
      "uri": "docjl://document/nonexistent_doc"
    }
  }')

if echo "$ERROR_RESPONSE" | grep -q '"error"'; then
  echo "✅ Error response for non-existent document"
else
  echo "❌ FAILED: Should return error for non-existent document"
  exit 1
fi

if echo "$ERROR_RESPONSE" | grep -q 'RESOURCE_NOT_FOUND\|not found'; then
  echo "✅ Correct error code/message"
else
  echo "❌ FAILED: Incorrect error code/message"
  exit 1
fi

echo ""
echo "============================================================"
echo "✅ ALL TESTS PASSED - resources/* handlers working correctly"
echo "============================================================"
