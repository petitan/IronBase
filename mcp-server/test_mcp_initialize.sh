#!/bin/bash
# Test MCP initialize handler

set -e

echo "============================================================"
echo "Test: MCP initialize Handler"
echo "============================================================"

# Test initialize
echo ""
echo "Test: initialize (MCP handshake)"
RESPONSE=$(curl -s -X POST http://127.0.0.1:8080/mcp \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc": "2.0",
    "id": 1,
    "method": "initialize",
    "params": {
      "protocolVersion": "2024-11-05",
      "capabilities": {},
      "clientInfo": {"name": "test-client", "version": "1.0"}
    }
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

# Check id field
if echo "$RESPONSE" | grep -q '"id": 1'; then
  echo "✅ id field correct"
else
  echo "❌ FAILED: Missing or incorrect id field"
  exit 1
fi

# Check protocolVersion
if echo "$RESPONSE" | grep -q '"protocolVersion": "2024-11-05"'; then
  echo "✅ protocolVersion correct (2024-11-05)"
else
  echo "❌ FAILED: Missing or incorrect protocolVersion"
  exit 1
fi

# Check capabilities
if echo "$RESPONSE" | grep -q '"capabilities"'; then
  echo "✅ capabilities field present"
else
  echo "❌ FAILED: Missing capabilities field"
  exit 1
fi

# Check tools capability
if echo "$RESPONSE" | grep -q '"tools"'; then
  echo "✅ tools capability declared"
else
  echo "❌ FAILED: Missing tools capability"
  exit 1
fi

# Check resources capability
if echo "$RESPONSE" | grep -q '"resources"'; then
  echo "✅ resources capability declared"
else
  echo "❌ FAILED: Missing resources capability"
  exit 1
fi

# Check prompts capability
if echo "$RESPONSE" | grep -q '"prompts"'; then
  echo "✅ prompts capability declared"
else
  echo "❌ FAILED: Missing prompts capability"
  exit 1
fi

# Check serverInfo
if echo "$RESPONSE" | grep -q '"serverInfo"'; then
  echo "✅ serverInfo field present"
else
  echo "❌ FAILED: Missing serverInfo field"
  exit 1
fi

# Check server name
if echo "$RESPONSE" | grep -q '"name": "docjl-editor"'; then
  echo "✅ server name correct"
else
  echo "❌ FAILED: Missing or incorrect server name"
  exit 1
fi

echo ""
echo "============================================================"
echo "✅ ALL TESTS PASSED - initialize handler working correctly"
echo "============================================================"
