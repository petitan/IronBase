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

echo "=== 1. Creating document via HTTP API ==="
curl -s -X POST http://127.0.0.1:8080/mcp \
  -H "Content-Type: application/json" \
  -d '{
    "method": "mcp_docjl_create_document",
    "params": {
      "document": {
        "id": "test_doc_1",
        "metadata": {"title": "Persistence Test", "version": "1.0"},
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
  }' | python3 -m json.tool

echo ""
echo "=== 2. List documents (before restart) ==="
curl -s -X POST http://127.0.0.1:8080/mcp \
  -H "Content-Type: application/json" \
  -d '{
    "method": "mcp_docjl_list_documents",
    "params": {}
  }' | python3 -m json.tool | tee /tmp/before.json

echo ""
echo "=== 3. Database files ==="
ls -lh docjl_storage.mlite docjl_storage.wal 2>&1

echo ""
echo "=== 4. Restarting server ==="
kill -9 $SERVER_PID
sleep 2
./target/release/mcp-docjl-server 2>&1 &
SERVER_PID=$!
sleep 3

echo "=== 5. List documents (after restart) ==="
curl -s -X POST http://127.0.0.1:8080/mcp \
  -H "Content-Type: application/json" \
  -d '{
    "method": "mcp_docjl_list_documents",
    "params": {}
  }' | python3 -m json.tool | tee /tmp/after.json

# Cleanup
kill -9 $SERVER_PID 2>/dev/null || true

echo ""
echo "=== TEST RESULTS ==="
BEFORE_COUNT=$(python3 -c "import json; d=json.load(open('/tmp/before.json')); print(len(d.get('result', {}).get('documents', [])))" 2>/dev/null || echo "0")
AFTER_COUNT=$(python3 -c "import json; d=json.load(open('/tmp/after.json')); print(len(d.get('result', {}).get('documents', [])))" 2>/dev/null || echo "0")

echo "Documents before restart: $BEFORE_COUNT"
echo "Documents after restart: $AFTER_COUNT"

if [ "$BEFORE_COUNT" == "$AFTER_COUNT" ] && [ "$BEFORE_COUNT" != "0" ]; then
  echo "✅ PERSISTENCE TEST PASSED!"
else
  echo "❌ PERSISTENCE TEST FAILED!"
  exit 1
fi
