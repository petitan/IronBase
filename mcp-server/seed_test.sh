#!/bin/bash

# Create test document with nested structure for delete testing

echo "Creating base document..."
curl -s -X POST http://localhost:8080/mcp -H "Content-Type: application/json" -d '{
  "jsonrpc": "2.0",
  "id": 1,
  "method": "mcp_docjl_insert_block",
  "params": {
    "document_id": "test_mcp",
    "position": "end",
    "block": {
      "type": "heading",
      "level": 1,
      "content": [{"type": "text", "content": "Chapter 1"}],
      "label": "sec:1"
    }
  }
}' | python3 -m json.tool

echo ""
echo "Adding para:1 inside sec:1..."
curl -s -X POST http://localhost:8080/mcp -H "Content-Type: application/json" -d '{
  "jsonrpc": "2.0",
  "id": 2,
  "method": "mcp_docjl_insert_block",
  "params": {
    "document_id": "test_mcp",
    "position": "inside:sec:1",
    "block": {
      "type": "paragraph",
      "content": [{"type": "text", "content": "First paragraph"}],
      "label": "para:1"
    }
  }
}' | python3 -m json.tool

echo ""
echo "Adding sec:1.1 inside sec:1..."
curl -s -X POST http://localhost:8080/mcp -H "Content-Type: application/json" -d '{
  "jsonrpc": "2.0",
  "id": 3,
  "method": "mcp_docjl_insert_block",
  "params": {
    "document_id": "test_mcp",
    "position": "inside:sec:1",
    "block": {
      "type": "heading",
      "level": 2,
      "content": [{"type": "text", "content": "Section 1.1"}],
      "label": "sec:1.1"
    }
  }
}' | python3 -m json.tool

echo ""
echo "Adding para:http inside sec:1.1..."
curl -s -X POST http://localhost:8080/mcp -H "Content-Type: application/json" -d '{
  "jsonrpc": "2.0",
  "id": 4,
  "method": "mcp_docjl_insert_block",
  "params": {
    "document_id": "test_mcp",
    "position": "inside:sec:1.1",
    "block": {
      "type": "paragraph",
      "content": [{"type": "text", "content": "HTTP paragraph"}],
      "label": "para:http"
    }
  }
}' | python3 -m json.tool

echo ""
echo "Adding sec:2..."
curl -s -X POST http://localhost:8080/mcp -H "Content-Type: application/json" -d '{
  "jsonrpc": "2.0",
  "id": 5,
  "method": "mcp_docjl_insert_block",
  "params": {
    "document_id": "test_mcp",
    "position": "end",
    "block": {
      "type": "heading",
      "level": 1,
      "content": [{"type": "text", "content": "Chapter 2"}],
      "label": "sec:2"
    }
  }
}' | python3 -m json.tool

echo ""
echo "Adding sec:special inside sec:2..."
curl -s -X POST http://localhost:8080/mcp -H "Content-Type: application/json" -d '{
  "jsonrpc": "2.0",
  "id": 6,
  "method": "mcp_docjl_insert_block",
  "params": {
    "document_id": "test_mcp",
    "position": "inside:sec:2",
    "block": {
      "type": "heading",
      "level": 2,
      "content": [{"type": "text", "content": "Special Section"}],
      "label": "sec:special"
    }
  }
}' | python3 -m json.tool

echo ""
echo "✅ Test document created! Structure:"
echo "- sec:1 (Chapter 1)"
echo "  - para:1 (First paragraph)"
echo "  - sec:1.1 (Section 1.1)"
echo "    - para:http (HTTP paragraph)"
echo "- sec:2 (Chapter 2)"
echo "  - sec:special (Special Section)"
