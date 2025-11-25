#!/bin/bash
set -e

# Clean up
killall -9 mcp-docjl-server 2>/dev/null || true
rm -f docjl_storage.mlite docjl_storage.wal
sleep 1

# Start server
./target/release/mcp-docjl-server 2>&1 &
SERVER_PID=$!
sleep 3

echo "=== Test 1: tools/call wrapper (MCP JSON-RPC protocol) ==="
curl -s -X POST http://127.0.0.1:8080/mcp \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc": "2.0",
    "id": 1,
    "method": "tools/call",
    "params": {
      "name": "mcp_docjl_create_document",
      "arguments": {
        "document": {
          "id": "doc_wrapper_test",
          "metadata": {"title": "Wrapper Test", "version": "1.0"},
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
  }' | python3 -m json.tool

echo ""
echo "=== Test 2: Direct method call (backward compatibility) ==="
curl -s -X POST http://127.0.0.1:8080/mcp \
  -H "Content-Type: application/json" \
  -d '{
    "method": "mcp_docjl_create_document",
    "params": {
      "document": {
        "id": "doc_direct_test",
        "metadata": {"title": "Direct Test", "version": "1.0"},
        "docjll": [
          {
            "type": "heading",
            "level": 1,
            "content": [{"type": "text", "content": "Direct Heading"}],
            "label": "sec:2",
            "children": []
          }
        ]
      }
    }
  }' | python3 -m json.tool

echo ""
echo "=== Test 3: List documents with tools/call wrapper ==="
curl -s -X POST http://127.0.0.1:8080/mcp \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc": "2.0",
    "id": 2,
    "method": "tools/call",
    "params": {
      "name": "mcp_docjl_list_documents"
    }
  }' | python3 -m json.tool

echo ""
echo "=== Test 4: List documents with direct method (backward compat) ==="
curl -s -X POST http://127.0.0.1:8080/mcp \
  -H "Content-Type: application/json" \
  -d '{
    "method": "mcp_docjl_list_documents",
    "params": {}
  }' | python3 -m json.tool

# Cleanup
kill -9 $SERVER_PID 2>/dev/null || true

echo ""
echo "=== All tests completed successfully! ==="
