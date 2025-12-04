#!/bin/bash
# Non-Repeatable Read Anomaly Test
# Tests if a reader sees different values for the same document during its "transaction"

MCP_URL="http://127.0.0.1:8080/mcp"

mcp() {
    curl -s -X POST "$MCP_URL" -H "Content-Type: application/json" -d "$1"
}

echo "╔══════════════════════════════════════════════════════════════╗"
echo "║     NON-REPEATABLE READ ANOMALY TEST                         ║"
echo "╚══════════════════════════════════════════════════════════════╝"
echo ""

# Setup
mcp '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"delete_many","arguments":{"collection":"nrr_test","filter":{}}}}' > /dev/null
mcp '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"insert_one","arguments":{"collection":"nrr_test","document":{"key":"data","value":100}}}}' > /dev/null

echo "Initial: value=100"
echo ""

# Writer thread: continuously updates the document
writer() {
    for i in $(seq 1 50); do
        mcp '{"jsonrpc":"2.0","id":'$i',"method":"tools/call","params":{"name":"update_one","arguments":{"collection":"nrr_test","filter":{"key":"data"},"update":{"$inc":{"value":1}}}}}' > /dev/null
        sleep 0.01
    done
}

# Reader thread: reads twice in quick succession and compares
reader() {
    local anomalies=0
    local total=0
    
    for i in $(seq 1 100); do
        # First read
        R1=$(mcp '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"find_one","arguments":{"collection":"nrr_test","filter":{"key":"data"}}}}')
        V1=$(echo "$R1" | python3 -c "import sys,json; d=json.load(sys.stdin); print(json.loads(d['result']['content'][0]['text'])['document']['value'])" 2>/dev/null)
        
        # Second read (immediate - simulating "same transaction")
        R2=$(mcp '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"find_one","arguments":{"collection":"nrr_test","filter":{"key":"data"}}}}')
        V2=$(echo "$R2" | python3 -c "import sys,json; d=json.load(sys.stdin); print(json.loads(d['result']['content'][0]['text'])['document']['value'])" 2>/dev/null)
        
        ((total++))
        
        if [[ "$V1" != "$V2" ]]; then
            ((anomalies++))
            echo "  ⚠️ Non-repeatable read: First=$V1, Second=$V2"
        fi
    done
    
    echo "NRR_ANOMALIES:$anomalies"
    echo "NRR_TOTAL:$total"
}

echo "┌─────────────────────────────────────────────────────────────┐"
echo "│  TEST: Reader reads twice, writer updates concurrently      │"
echo "└─────────────────────────────────────────────────────────────┘"

START=$(date +%s.%N)

# Run writer and reader concurrently
writer &
PW=$!
RESULT=$(reader)
wait $PW

END=$(date +%s.%N)

# Parse results
ANOMALIES=$(echo "$RESULT" | grep "NRR_ANOMALIES:" | cut -d: -f2)
TOTAL=$(echo "$RESULT" | grep "NRR_TOTAL:" | cut -d: -f2)

# Final value check
FINAL=$(mcp '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"find_one","arguments":{"collection":"nrr_test","filter":{"key":"data"}}}}')
FINAL_VAL=$(echo "$FINAL" | python3 -c "import sys,json; d=json.load(sys.stdin); print(json.loads(d['result']['content'][0]['text'])['document']['value'])" 2>/dev/null)

echo ""
echo "╔══════════════════════════════════════════════════════════════╗"
echo "║                        RESULTS                               ║"
echo "╚══════════════════════════════════════════════════════════════╝"
echo "  Total read pairs: $TOTAL"
echo "  Non-repeatable reads: $ANOMALIES"
echo "  Final value: $FINAL_VAL (expected: 150)"
echo "  Time: $(echo "$END - $START" | bc)s"
echo ""

if [[ "$ANOMALIES" == "0" ]]; then
    echo "  ✅ PASS - No non-repeatable reads detected!"
    echo "  (Note: This is expected with READ COMMITTED isolation)"
else
    echo "  ⚠️ INFO - $ANOMALIES non-repeatable reads detected"
    echo "  (This is normal behavior without REPEATABLE READ isolation)"
fi
