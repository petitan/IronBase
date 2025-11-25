#!/bin/bash
# Test MCP prompts/list handler

set -e

echo "============================================================"
echo "Test: MCP prompts/list Handler"
echo "============================================================"

# Test prompts/list
echo ""
echo "Test: prompts/list (15 prompts: 10 Balanced + 5 Calibration)"
RESPONSE=$(curl -s -X POST http://127.0.0.1:8080/mcp \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc": "2.0",
    "id": 1,
    "method": "prompts/list",
    "params": {}
  }')

echo "$RESPONSE" | python3 -m json.tool | head -50

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

# Check prompts array
if echo "$RESPONSE" | grep -q '"prompts"'; then
  echo "✅ prompts field present"
else
  echo "❌ FAILED: Missing prompts field"
  exit 1
fi

# Count prompts (should be 15)
PROMPT_COUNT=$(echo "$RESPONSE" | python3 -c "import sys, json; d=json.load(sys.stdin); print(len(d['result']['prompts']))")
if [ "$PROMPT_COUNT" -eq 15 ]; then
  echo "✅ Prompt count correct: 15 prompts"
else
  echo "❌ FAILED: Expected 15 prompts, got $PROMPT_COUNT"
  exit 1
fi

# Check specific prompts exist (sample from each category)
echo ""
echo "Checking Balanced MVP prompts (10)..."
BALANCED_PROMPTS=(
  "validate-structure"
  "validate-compliance"
  "create-section"
  "summarize-document"
  "suggest-improvements"
  "audit-readiness"
  "create-outline"
  "analyze-changes"
  "check-consistency"
  "resolve-reference"
)

for PROMPT in "${BALANCED_PROMPTS[@]}"; do
  if echo "$RESPONSE" | grep -q "\"name\": \"$PROMPT\""; then
    echo "✅ Balanced prompt found: $PROMPT"
  else
    echo "❌ FAILED: Missing balanced prompt: $PROMPT"
    exit 1
  fi
done

echo ""
echo "Checking Calibration-specific prompts (5)..."
CALIBRATION_PROMPTS=(
  "calculate-measurement-uncertainty"
  "generate-calibration-hierarchy"
  "determine-calibration-interval"
  "create-calibration-certificate"
  "generate-uncertainty-budget"
)

for PROMPT in "${CALIBRATION_PROMPTS[@]}"; do
  if echo "$RESPONSE" | grep -q "\"name\": \"$PROMPT\""; then
    echo "✅ Calibration prompt found: $PROMPT"
  else
    echo "❌ FAILED: Missing calibration prompt: $PROMPT"
    exit 1
  fi
done

# Check that each prompt has name, description, and arguments
echo ""
echo "Validating prompt schemas..."
PROMPTS_WITH_SCHEMA=$(echo "$RESPONSE" | python3 -c "
import sys, json
d = json.load(sys.stdin)
prompts = d['result']['prompts']
valid = 0
for prompt in prompts:
    if 'name' in prompt and 'description' in prompt and 'arguments' in prompt:
        valid += 1
print(valid)
")

if [ "$PROMPTS_WITH_SCHEMA" -eq 15 ]; then
  echo "✅ All 15 prompts have name, description, and arguments"
else
  echo "❌ FAILED: Only $PROMPTS_WITH_SCHEMA/15 prompts have complete schema"
  exit 1
fi

echo ""
echo "============================================================"
echo "✅ ALL TESTS PASSED - prompts/list handler working correctly"
echo "============================================================"
