#!/bin/bash
# Integration test: Full MCP protocol flow
# Tests: initialize → tools/list → tools/call → resources/list → resources/read → prompts/list

set -e

echo "============================================================"
echo "Integration Test: Full MCP Protocol Flow"
echo "============================================================"

# Step 1: initialize
echo ""
echo "Step 1: MCP Handshake (initialize)"
echo "------------------------------------------------------------"
INIT_RESPONSE=$(curl -s -X POST http://127.0.0.1:8080/mcp \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc": "2.0",
    "id": 1,
    "method": "initialize",
    "params": {
      "protocolVersion": "2024-11-05",
      "capabilities": {},
      "clientInfo": {"name": "integration-test", "version": "1.0"}
    }
  }')

if echo "$INIT_RESPONSE" | grep -q '"protocolVersion": "2024-11-05"'; then
  echo "✅ initialize successful (protocol 2024-11-05)"
else
  echo "❌ FAILED: initialize"
  exit 1
fi

# Step 2: tools/list
echo ""
echo "Step 2: Discover Available Tools (tools/list)"
echo "------------------------------------------------------------"
TOOLS_RESPONSE=$(curl -s -X POST http://127.0.0.1:8080/mcp \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc": "2.0",
    "id": 2,
    "method": "tools/list",
    "params": {}
  }')

TOOL_COUNT=$(echo "$TOOLS_RESPONSE" | python3 -c "import sys, json; d=json.load(sys.stdin); print(len(d['result']['tools']))")
if [ "$TOOL_COUNT" -eq 9 ]; then
  echo "✅ tools/list successful (9 tools available)"
else
  echo "❌ FAILED: tools/list (expected 9 tools, got $TOOL_COUNT)"
  exit 1
fi

# Step 3: tools/call (create document)
echo ""
echo "Step 3: Execute Tool (tools/call - create_document)"
echo "------------------------------------------------------------"
CREATE_RESPONSE=$(curl -s -X POST http://127.0.0.1:8080/mcp \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc": "2.0",
    "id": 3,
    "method": "tools/call",
    "params": {
      "name": "mcp_docjl_create_document",
      "arguments": {
        "document": {
          "id": "integration_test_doc",
          "metadata": {"title": "Integration Test Document", "version": "1.0"},
          "docjll": [
            {
              "type": "heading",
              "level": 1,
              "content": [{"type": "text", "content": "Integration Test"}],
              "label": "sec:1",
              "children": [
                {
                  "type": "paragraph",
                  "content": [{"type": "text", "content": "This document was created by the integration test."}],
                  "label": "para:1"
                }
              ]
            }
          ]
        }
      }
    }
  }')

if echo "$CREATE_RESPONSE" | grep -q '"success": true'; then
  echo "✅ tools/call successful (document created)"
else
  echo "❌ FAILED: tools/call (create_document)"
  exit 1
fi

# Step 4: resources/list
echo ""
echo "Step 4: List Available Resources (resources/list)"
echo "------------------------------------------------------------"
RESOURCES_RESPONSE=$(curl -s -X POST http://127.0.0.1:8080/mcp \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc": "2.0",
    "id": 4,
    "method": "resources/list",
    "params": {}
  }')

if echo "$RESOURCES_RESPONSE" | grep -q '"uri": "docjl://document/integration_test_doc"'; then
  RESOURCE_COUNT=$(echo "$RESOURCES_RESPONSE" | python3 -c "import sys, json; d=json.load(sys.stdin); print(len(d['result']['resources']))")
  echo "✅ resources/list successful (found $RESOURCE_COUNT resource(s) including new document)"
else
  echo "❌ FAILED: resources/list (document not found)"
  exit 1
fi

# Step 5: resources/read
echo ""
echo "Step 5: Read Resource (resources/read)"
echo "------------------------------------------------------------"
READ_RESPONSE=$(curl -s -X POST http://127.0.0.1:8080/mcp \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc": "2.0",
    "id": 5,
    "method": "resources/read",
    "params": {
      "uri": "docjl://document/integration_test_doc"
    }
  }')

if echo "$READ_RESPONSE" | grep -q '"contents"' && echo "$READ_RESPONSE" | grep -q '"text"'; then
  echo "✅ resources/read successful (document content retrieved)"
else
  echo "❌ FAILED: resources/read"
  exit 1
fi

# Step 6: prompts/list
echo ""
echo "Step 6: List Available Prompts (prompts/list)"
echo "------------------------------------------------------------"
PROMPTS_RESPONSE=$(curl -s -X POST http://127.0.0.1:8080/mcp \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc": "2.0",
    "id": 6,
    "method": "prompts/list",
    "params": {}
  }')

PROMPT_COUNT=$(echo "$PROMPTS_RESPONSE" | python3 -c "import sys, json; d=json.load(sys.stdin); print(len(d['result']['prompts']))")
if [ "$PROMPT_COUNT" -eq 15 ]; then
  echo "✅ prompts/list successful (15 prompts available)"
else
  echo "❌ FAILED: prompts/list (expected 15 prompts, got $PROMPT_COUNT)"
  exit 1
fi

# Step 7: Additional tools/call operations (list, get, search)
echo ""
echo "Step 7: Additional Tool Calls (list, get, search)"
echo "------------------------------------------------------------"

# List documents
LIST_RESPONSE=$(curl -s -X POST http://127.0.0.1:8080/mcp \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc": "2.0",
    "id": 7,
    "method": "tools/call",
    "params": {
      "name": "mcp_docjl_list_documents"
    }
  }')

if echo "$LIST_RESPONSE" | grep -q '"integration_test_doc"'; then
  echo "✅ list_documents successful"
else
  echo "❌ FAILED: list_documents"
  exit 1
fi

# Get document
GET_RESPONSE=$(curl -s -X POST http://127.0.0.1:8080/mcp \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc": "2.0",
    "id": 8,
    "method": "tools/call",
    "params": {
      "name": "mcp_docjl_get_document",
      "arguments": {
        "document_id": "integration_test_doc"
      }
    }
  }')

if echo "$GET_RESPONSE" | grep -q '"id": "integration_test_doc"'; then
  echo "✅ get_document successful"
else
  echo "❌ FAILED: get_document"
  exit 1
fi

# Search content
SEARCH_RESPONSE=$(curl -s -X POST http://127.0.0.1:8080/mcp \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc": "2.0",
    "id": 9,
    "method": "tools/call",
    "params": {
      "name": "mcp_docjl_search_content",
      "arguments": {
        "document_id": "integration_test_doc",
        "query": "integration"
      }
    }
  }')

if echo "$SEARCH_RESPONSE" | grep -q '"results"'; then
  echo "✅ search_content successful"
else
  echo "❌ FAILED: search_content"
  exit 1
fi

# Summary
echo ""
echo "============================================================"
echo "✅ INTEGRATION TEST PASSED - Full MCP flow working"
echo "============================================================"
echo ""
echo "Test Summary:"
echo "  1. initialize ✅"
echo "  2. tools/list (9 tools) ✅"
echo "  3. tools/call (create_document) ✅"
echo "  4. resources/list ✅"
echo "  5. resources/read ✅"
echo "  6. prompts/list (15 prompts) ✅"
echo "  7. Additional tool calls (list, get, search) ✅"
echo ""
echo "All MCP protocol methods working correctly!"
echo "============================================================"
