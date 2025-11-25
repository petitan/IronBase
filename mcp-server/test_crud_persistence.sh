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

echo "=== 1. Creating multiple documents ==="
curl -s -X POST http://127.0.0.1:8080/mcp \
  -H "Content-Type: application/json" \
  -d '{
    "method": "mcp_docjl_create_document",
    "params": {
      "document": {
        "id": "doc_1",
        "metadata": {"title": "First Document", "version": "1.0"},
        "docjll": [
          {
            "type": "heading",
            "level": 1,
            "content": [{"type": "text", "content": "First Heading"}],
            "label": "sec:1",
            "children": []
          }
        ]
      }
    }
  }' | python3 -m json.tool

curl -s -X POST http://127.0.0.1:8080/mcp \
  -H "Content-Type: application/json" \
  -d '{
    "method": "mcp_docjl_create_document",
    "params": {
      "document": {
        "id": "doc_2",
        "metadata": {"title": "Second Document", "version": "2.0"},
        "docjll": [
          {
            "type": "heading",
            "level": 1,
            "content": [{"type": "text", "content": "Second Heading"}],
            "label": "sec:2",
            "children": []
          }
        ]
      }
    }
  }' | python3 -m json.tool

echo ""
echo "=== 2. Get specific document (before restart) ==="
curl -s -X POST http://127.0.0.1:8080/mcp \
  -H "Content-Type: application/json" \
  -d '{
    "method": "mcp_docjl_get_document",
    "params": {"document_id": "doc_1"}
  }' | python3 -m json.tool | tee /tmp/get_before.json

echo ""
echo "=== 3. List all documents (before restart) ==="
curl -s -X POST http://127.0.0.1:8080/mcp \
  -H "Content-Type: application/json" \
  -d '{
    "method": "mcp_docjl_list_documents",
    "params": {}
  }' | python3 -m json.tool | tee /tmp/list_before.json

echo ""
echo "=== 4. Database files ==="
ls -lh docjl_storage.mlite docjl_storage.wal 2>&1

echo ""
echo "=== 5. Restarting server ==="
kill -9 $SERVER_PID
sleep 2
./target/release/mcp-docjl-server 2>&1 &
SERVER_PID=$!
sleep 3

echo "=== 6. Get specific document (after restart) ==="
curl -s -X POST http://127.0.0.1:8080/mcp \
  -H "Content-Type: application/json" \
  -d '{
    "method": "mcp_docjl_get_document",
    "params": {"document_id": "doc_1"}
  }' | python3 -m json.tool | tee /tmp/get_after.json

echo ""
echo "=== 7. List all documents (after restart) ==="
curl -s -X POST http://127.0.0.1:8080/mcp \
  -H "Content-Type: application/json" \
  -d '{
    "method": "mcp_docjl_list_documents",
    "params": {}
  }' | python3 -m json.tool | tee /tmp/list_after.json

# Cleanup
kill -9 $SERVER_PID 2>/dev/null || true

echo ""
echo "=== TEST RESULTS ==="

# Count documents before and after
LIST_BEFORE=$(python3 -c "import json; d=json.load(open('/tmp/list_before.json')); print(len(d.get('result', {}).get('documents', [])))" 2>/dev/null || echo "0")
LIST_AFTER=$(python3 -c "import json; d=json.load(open('/tmp/list_after.json')); print(len(d.get('result', {}).get('documents', [])))" 2>/dev/null || echo "0")

# Check if get_document returns same data
GET_BEFORE_ID=$(python3 -c "import json; d=json.load(open('/tmp/get_before.json')); print(d.get('result', {}).get('document', {}).get('id', ''))" 2>/dev/null || echo "")
GET_AFTER_ID=$(python3 -c "import json; d=json.load(open('/tmp/get_after.json')); print(d.get('result', {}).get('document', {}).get('id', ''))" 2>/dev/null || echo "")

GET_BEFORE_TITLE=$(python3 -c "import json; d=json.load(open('/tmp/get_before.json')); print(d.get('result', {}).get('document', {}).get('metadata', {}).get('title', ''))" 2>/dev/null || echo "")
GET_AFTER_TITLE=$(python3 -c "import json; d=json.load(open('/tmp/get_after.json')); print(d.get('result', {}).get('document', {}).get('metadata', {}).get('title', ''))" 2>/dev/null || echo "")

echo "List documents before restart: $LIST_BEFORE"
echo "List documents after restart: $LIST_AFTER"
echo ""
echo "Get document before restart: id='$GET_BEFORE_ID', title='$GET_BEFORE_TITLE'"
echo "Get document after restart: id='$GET_AFTER_ID', title='$GET_AFTER_TITLE'"

# Check all conditions
PASS=true

if [ "$LIST_BEFORE" != "$LIST_AFTER" ] || [ "$LIST_BEFORE" != "2" ]; then
  echo "❌ List documents count mismatch!"
  PASS=false
fi

if [ "$GET_BEFORE_ID" != "$GET_AFTER_ID" ] || [ "$GET_BEFORE_ID" != "doc_1" ]; then
  echo "❌ Get document ID mismatch!"
  PASS=false
fi

if [ "$GET_BEFORE_TITLE" != "$GET_AFTER_TITLE" ] || [ "$GET_BEFORE_TITLE" != "First Document" ]; then
  echo "❌ Get document title mismatch!"
  PASS=false
fi

if [ "$PASS" = true ]; then
  echo ""
  echo "✅ ALL CRUD PERSISTENCE TESTS PASSED!"
else
  echo ""
  echo "❌ CRUD PERSISTENCE TEST FAILED!"
  exit 1
fi
