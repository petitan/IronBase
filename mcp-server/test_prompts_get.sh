#!/bin/bash
# Test prompts/get endpoint implementation

set -e

echo "============================================================"
echo "MCP prompts/get Endpoint Test"
echo "============================================================"

echo ""
echo "=== Test 1: Get valid prompt (validate-structure) ==="
RESPONSE=$(curl -s -X POST http://127.0.0.1:8080/mcp \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc": "2.0",
    "id": 1,
    "method": "prompts/get",
    "params": {"name": "validate-structure"}
  }')

if echo "$RESPONSE" | grep -q '"name": "validate-structure"'; then
  echo "✅ Test 1 PASSED: Retrieved valid prompt"
  echo "$RESPONSE" | python3 -m json.tool
else
  echo "❌ Test 1 FAILED"
  exit 1
fi

echo ""
echo "=== Test 2: Get non-existent prompt ==="
RESPONSE=$(curl -s -X POST http://127.0.0.1:8080/mcp \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc": "2.0",
    "id": 2,
    "method": "prompts/get",
    "params": {"name": "non-existent-prompt"}
  }')

if echo "$RESPONSE" | grep -q '"code": "PROMPT_NOT_FOUND"'; then
  echo "✅ Test 2 PASSED: Correct 404 error"
  echo "$RESPONSE" | python3 -m json.tool
else
  echo "❌ Test 2 FAILED"
  exit 1
fi

echo ""
echo "=== Test 3: Missing 'name' parameter ==="
RESPONSE=$(curl -s -X POST http://127.0.0.1:8080/mcp \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc": "2.0",
    "id": 3,
    "method": "prompts/get",
    "params": {}
  }')

if echo "$RESPONSE" | grep -q '"code": "INVALID_PARAMS"'; then
  echo "✅ Test 3 PASSED: Correct parameter validation"
  echo "$RESPONSE" | python3 -m json.tool
else
  echo "❌ Test 3 FAILED"
  exit 1
fi

echo ""
echo "=== Test 4: Get ISO 17025 calibration prompt ==="
RESPONSE=$(curl -s -X POST http://127.0.0.1:8080/mcp \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc": "2.0",
    "id": 4,
    "method": "prompts/get",
    "params": {"name": "generate-calibration-hierarchy"}
  }')

if echo "$RESPONSE" | grep -q '"name": "generate-calibration-hierarchy"' && echo "$RESPONSE" | grep -q '"instrument_type"'; then
  echo "✅ Test 4 PASSED: ISO 17025 calibration prompt retrieved"
  echo "$RESPONSE" | python3 -m json.tool
else
  echo "❌ Test 4 FAILED"
  exit 1
fi

echo ""
echo "============================================================"
echo "✅ ALL TESTS PASSED"
echo "============================================================"
echo ""
echo "Summary:"
echo "  1. Valid prompt retrieval ✅"
echo "  2. 404 error handling ✅"
echo "  3. Parameter validation ✅"
echo "  4. ISO 17025 prompts ✅"
echo ""
echo "prompts/get endpoint is PRODUCTION READY! 🎉"
echo "============================================================"
