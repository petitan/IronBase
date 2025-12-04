#!/bin/bash
# Phantom Read Anomaly Test
# Tests if a query returns different number of rows when re-executed
# because another transaction inserted/deleted documents matching the filter

MCP_URL="http://127.0.0.1:8080/mcp"

mcp() {
    curl -s -X POST "$MCP_URL" -H "Content-Type: application/json" -d "$1"
}

echo "╔══════════════════════════════════════════════════════════════╗"
echo "║           PHANTOM READ ANOMALY TEST                          ║"
echo "╚══════════════════════════════════════════════════════════════╝"
echo ""

# Setup: Create initial documents with status="active"
mcp '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"delete_many","arguments":{"collection":"phantom_test","filter":{}}}}' > /dev/null

for i in $(seq 1 10); do
    mcp '{"jsonrpc":"2.0","id":'$i',"method":"tools/call","params":{"name":"insert_one","arguments":{"collection":"phantom_test","document":{"item":'$i',"status":"active"}}}}' > /dev/null
done

echo "Initial: 10 documents with status='active'"
echo ""

# Writer thread: continuously inserts and deletes documents
writer() {
    for i in $(seq 1 30); do
        # Insert a new "active" document
        NEW_ID=$((100 + i))
        mcp '{"jsonrpc":"2.0","id":'$i',"method":"tools/call","params":{"name":"insert_one","arguments":{"collection":"phantom_test","document":{"item":'$NEW_ID',"status":"active"}}}}' > /dev/null
        sleep 0.02
        # Delete it
        mcp '{"jsonrpc":"2.0","id":'$i',"method":"tools/call","params":{"name":"delete_one","arguments":{"collection":"phantom_test","filter":{"item":'$NEW_ID'}}}}' > /dev/null
        sleep 0.02
    done
}

# Reader thread: counts "active" documents twice in quick succession
reader() {
    local phantoms=0
    local total=0
    
    for i in $(seq 1 50); do
        # First count
        R1=$(mcp '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"find","arguments":{"collection":"phantom_test","filter":{"status":"active"}}}}')
        C1=$(echo "$R1" | grep -o '"item"' | wc -l)
        
        # Second count (immediate)
        R2=$(mcp '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"find","arguments":{"collection":"phantom_test","filter":{"status":"active"}}}}')
        C2=$(echo "$R2" | grep -o '"item"' | wc -l)
        
        ((total++))
        
        if [[ "$C1" != "$C2" ]]; then
            ((phantoms++))
            echo "  👻 Phantom read: First count=$C1, Second count=$C2 (diff=$((C2 - C1)))"
        fi
    done
    
    echo "PHANTOM_COUNT:$phantoms"
    echo "TOTAL_CHECKS:$total"
}

echo "┌─────────────────────────────────────────────────────────────┐"
echo "│  TEST: Reader counts twice, writer inserts/deletes         │"
echo "│  concurrently (documents appearing/disappearing)           │"
echo "└─────────────────────────────────────────────────────────────┘"

START=$(date +%s.%N)

# Run writer and reader concurrently
writer &
PW=$!
RESULT=$(reader)
wait $PW

END=$(date +%s.%N)

# Parse results
PHANTOMS=$(echo "$RESULT" | grep "PHANTOM_COUNT:" | cut -d: -f2)
TOTAL=$(echo "$RESULT" | grep "TOTAL_CHECKS:" | cut -d: -f2)

# Final count
FINAL=$(mcp '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"find","arguments":{"collection":"phantom_test","filter":{"status":"active"}}}}')
FINAL_COUNT=$(echo "$FINAL" | grep -o '"item"' | wc -l)

echo ""
echo "╔══════════════════════════════════════════════════════════════╗"
echo "║                        RESULTS                               ║"
echo "╚══════════════════════════════════════════════════════════════╝"
echo "  Total query pairs: $TOTAL"
echo "  Phantom reads detected: $PHANTOMS"
echo "  Final document count: $FINAL_COUNT (expected: 10)"
echo "  Time: $(echo "$END - $START" | bc)s"
echo ""

if [[ "$PHANTOMS" == "0" ]]; then
    echo "  ✅ PASS - No phantom reads detected!"
    echo "  (Note: This might happen if timing didn't align)"
else
    echo "  👻 INFO - $PHANTOMS phantom reads detected"
    echo "  (This is normal behavior without SERIALIZABLE isolation)"
fi

echo ""
echo "┌─────────────────────────────────────────────────────────────┐"
echo "│  EXPLANATION                                                │"
echo "│  Phantom reads occur when:                                  │"
echo "│  - Query 1: SELECT * WHERE status='active' → 10 rows       │"
echo "│  - Another tx: INSERT new active document                   │"
echo "│  - Query 2: SELECT * WHERE status='active' → 11 rows       │"
echo "│  The 'phantom' row appeared between identical queries!      │"
echo "└─────────────────────────────────────────────────────────────┘"
