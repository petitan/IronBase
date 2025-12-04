#!/bin/bash
# Read Skew & Write Skew Anomaly Tests
MCP_URL="http://127.0.0.1:8080/mcp"

mcp() {
    curl -s -X POST "$MCP_URL" -H "Content-Type: application/json" -d "$1"
}

get_val() {
    local coll=$1
    local key=$2
    local r=$(mcp '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"find_one","arguments":{"collection":"'$coll'","filter":{"key":"'$key'"}}}}')
    echo "$r" | python3 -c "import sys,json; d=json.load(sys.stdin); doc=json.loads(d['result']['content'][0]['text'])['document']; print(doc.get('value', 'null'))" 2>/dev/null
}

echo "╔══════════════════════════════════════════════════════════════╗"
echo "║        READ SKEW & WRITE SKEW ANOMALY TESTS                  ║"
echo "╚══════════════════════════════════════════════════════════════╝"
echo ""

#############################################
# TEST 1: READ SKEW
#############################################
echo "┌─────────────────────────────────────────────────────────────┐"
echo "│  TEST 1: READ SKEW                                         │"
echo "│  Invariant: A + B = 100 always                             │"
echo "│  Read Skew: Reader sees A=50 (before), B=60 (after update) │"
echo "└─────────────────────────────────────────────────────────────┘"

# Setup: A=50, B=50 (sum=100)
mcp '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"delete_many","arguments":{"collection":"read_skew","filter":{}}}}' > /dev/null
mcp '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"insert_one","arguments":{"collection":"read_skew","document":{"key":"A","value":50}}}}' > /dev/null
mcp '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"insert_one","arguments":{"collection":"read_skew","document":{"key":"B","value":50}}}}' > /dev/null

echo "Initial: A=50, B=50 (sum=100)"

# Writer: continuously transfers between A and B (maintaining sum=100)
writer_read_skew() {
    for i in $(seq 1 100); do
        AMT=$((RANDOM % 20 + 1))
        # Transfer from A to B
        mcp '{"jsonrpc":"2.0","id":'$i',"method":"tools/call","params":{"name":"update_one","arguments":{"collection":"read_skew","filter":{"key":"A"},"update":{"$inc":{"value":-'$AMT'}}}}}' > /dev/null
        mcp '{"jsonrpc":"2.0","id":'$i',"method":"tools/call","params":{"name":"update_one","arguments":{"collection":"read_skew","filter":{"key":"B"},"update":{"$inc":{"value":'$AMT'}}}}}' > /dev/null
        # Transfer back
        mcp '{"jsonrpc":"2.0","id":'$i',"method":"tools/call","params":{"name":"update_one","arguments":{"collection":"read_skew","filter":{"key":"A"},"update":{"$inc":{"value":'$AMT'}}}}}' > /dev/null
        mcp '{"jsonrpc":"2.0","id":'$i',"method":"tools/call","params":{"name":"update_one","arguments":{"collection":"read_skew","filter":{"key":"B"},"update":{"$inc":{"value":-'$AMT'}}}}}' > /dev/null
    done
}

# Reader: reads A, then B - checks if sum=100
reader_read_skew() {
    local skews=0
    local total=0

    for i in $(seq 1 150); do
        A=$(get_val "read_skew" "A")
        B=$(get_val "read_skew" "B")

        ((total++))

        if [[ -n "$A" && -n "$B" && "$A" != "null" && "$B" != "null" ]]; then
            SUM=$((A + B))
            if [[ "$SUM" -ne 100 ]]; then
                ((skews++))
                echo "  📖 Read skew #$skews: A=$A, B=$B, Sum=$SUM (expected 100)"
            fi
        fi
    done

    echo "READ_SKEW_COUNT:$skews"
    echo "READ_SKEW_TOTAL:$total"
}

START=$(date +%s.%N)

writer_read_skew &
PW=$!
RESULT=$(reader_read_skew)
wait $PW

END=$(date +%s.%N)

SKEWS=$(echo "$RESULT" | grep "READ_SKEW_COUNT:" | cut -d: -f2)
TOTAL=$(echo "$RESULT" | grep "READ_SKEW_TOTAL:" | cut -d: -f2)

# Final check
FA=$(get_val "read_skew" "A")
FB=$(get_val "read_skew" "B")

echo ""
echo "  Total reads: $TOTAL"
echo "  Read skews detected: $SKEWS"
echo "  Final: A=$FA, B=$FB, Sum=$((FA + FB))"
echo "  Time: $(echo "$END - $START" | bc)s"

if [[ "$SKEWS" == "0" ]]; then
    echo "  ✅ PASS - No read skew detected!"
else
    echo "  📖 INFO - $SKEWS read skews detected"
    echo "  (Expected without SNAPSHOT isolation)"
fi

echo ""

#############################################
# TEST 2: WRITE SKEW
#############################################
echo "┌─────────────────────────────────────────────────────────────┐"
echo "│  TEST 2: WRITE SKEW                                        │"
echo "│  Constraint: At least 1 doctor must be on-call             │"
echo "│  Write Skew: Both doctors read '2 on-call', both go off    │"
echo "└─────────────────────────────────────────────────────────────┘"

# Setup: 2 doctors, both on-call
mcp '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"delete_many","arguments":{"collection":"write_skew","filter":{}}}}' > /dev/null
mcp '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"insert_one","arguments":{"collection":"write_skew","document":{"doctor":"Alice","on_call":true}}}}' > /dev/null
mcp '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"insert_one","arguments":{"collection":"write_skew","document":{"doctor":"Bob","on_call":true}}}}' > /dev/null

echo "Initial: Alice=on_call, Bob=on_call (2 doctors on-call)"

VIOLATIONS=0
ROUNDS=30

for round in $(seq 1 $ROUNDS); do
    # Reset: both on-call
    mcp '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"update_one","arguments":{"collection":"write_skew","filter":{"doctor":"Alice"},"update":{"$set":{"on_call":true}}}}}' > /dev/null
    mcp '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"update_one","arguments":{"collection":"write_skew","filter":{"doctor":"Bob"},"update":{"$set":{"on_call":true}}}}}' > /dev/null

    # Simulate write skew: both try to go off-call if >1 on-call
    # Alice's transaction (simulated)
    alice_tx() {
        # Read: count on-call doctors
        local count=$(mcp '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"find","arguments":{"collection":"write_skew","filter":{"on_call":true}}}}' | python3 -c "import sys,json; r=json.load(sys.stdin); print(len(json.loads(r['result']['content'][0]['text'])['documents']))" 2>/dev/null)
        # If >1, I can go off-call
        if [[ "$count" -gt 1 ]]; then
            mcp '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"update_one","arguments":{"collection":"write_skew","filter":{"doctor":"Alice"},"update":{"$set":{"on_call":false}}}}}' > /dev/null
        fi
    }

    # Bob's transaction (simulated)
    bob_tx() {
        # Read: count on-call doctors
        local count=$(mcp '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"find","arguments":{"collection":"write_skew","filter":{"on_call":true}}}}' | python3 -c "import sys,json; r=json.load(sys.stdin); print(len(json.loads(r['result']['content'][0]['text'])['documents']))" 2>/dev/null)
        # If >1, I can go off-call
        if [[ "$count" -gt 1 ]]; then
            mcp '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"update_one","arguments":{"collection":"write_skew","filter":{"doctor":"Bob"},"update":{"$set":{"on_call":false}}}}}' > /dev/null
        fi
    }

    # Run both "transactions" concurrently
    alice_tx &
    PA=$!
    bob_tx &
    PB=$!
    wait $PA $PB

    # Check: is anyone on-call?
    FINAL_COUNT=$(mcp '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"find","arguments":{"collection":"write_skew","filter":{"on_call":true}}}}' | python3 -c "import sys,json; r=json.load(sys.stdin); print(len(json.loads(r['result']['content'][0]['text'])['documents']))" 2>/dev/null)

    if [[ "$FINAL_COUNT" == "0" ]]; then
        ((VIOLATIONS++))
        echo "  ⚠️ Write skew in round $round: 0 doctors on-call!"
    fi
done

echo ""
echo "  Total rounds: $ROUNDS"
echo "  Write skew violations: $VIOLATIONS"

if [[ "$VIOLATIONS" == "0" ]]; then
    echo "  ✅ PASS - No write skew detected!"
    echo "  (Note: timing may not have aligned for race)"
else
    echo "  ⚠️ INFO - $VIOLATIONS write skew violations"
    echo "  (Expected without SERIALIZABLE isolation)"
fi

echo ""

#############################################
# TEST 3: WRITE SKEW (More aggressive)
#############################################
echo "┌─────────────────────────────────────────────────────────────┐"
echo "│  TEST 3: WRITE SKEW (Aggressive - inventory constraint)    │"
echo "│  Constraint: stock >= 0 always                             │"
echo "│  Both threads check stock=10, both try to take 8           │"
echo "└─────────────────────────────────────────────────────────────┘"

VIOLATIONS2=0
ROUNDS2=30

for round in $(seq 1 $ROUNDS2); do
    # Reset: stock=10
    mcp '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"delete_many","arguments":{"collection":"inventory","filter":{}}}}' > /dev/null
    mcp '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"insert_one","arguments":{"collection":"inventory","document":{"item":"widget","stock":10}}}}' > /dev/null

    # Both try to take 8 items if stock >= 8
    take_items() {
        local buyer=$1
        local stock=$(mcp '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"find_one","arguments":{"collection":"inventory","filter":{"item":"widget"}}}}' | python3 -c "import sys,json; d=json.load(sys.stdin); doc=json.loads(d['result']['content'][0]['text'])['document']; print(doc.get('stock', 0))" 2>/dev/null)
        if [[ "$stock" -ge 8 ]]; then
            mcp '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"update_one","arguments":{"collection":"inventory","filter":{"item":"widget"},"update":{"$inc":{"stock":-8}}}}}' > /dev/null
        fi
    }

    take_items "Buyer1" &
    P1=$!
    take_items "Buyer2" &
    P2=$!
    wait $P1 $P2

    # Check final stock
    FINAL_STOCK=$(mcp '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"find_one","arguments":{"collection":"inventory","filter":{"item":"widget"}}}}' | python3 -c "import sys,json; d=json.load(sys.stdin); doc=json.loads(d['result']['content'][0]['text'])['document']; print(doc.get('stock', 0))" 2>/dev/null)

    if [[ "$FINAL_STOCK" -lt 0 ]]; then
        ((VIOLATIONS2++))
        echo "  ⚠️ Write skew in round $round: stock=$FINAL_STOCK (negative!)"
    fi
done

echo ""
echo "  Total rounds: $ROUNDS2"
echo "  Negative stock violations: $VIOLATIONS2"

if [[ "$VIOLATIONS2" == "0" ]]; then
    echo "  ✅ PASS - No negative stock!"
else
    echo "  ⚠️ INFO - $VIOLATIONS2 negative stock violations"
    echo "  (This is a classic write skew scenario)"
fi

echo ""
echo "╔══════════════════════════════════════════════════════════════╗"
echo "║                        SUMMARY                               ║"
echo "╚══════════════════════════════════════════════════════════════╝"
echo "  Read Skew:   Detected $SKEWS times (expected in READ COMMITTED)"
echo "  Write Skew:  Doctor test: $VIOLATIONS violations"
echo "  Write Skew:  Inventory test: $VIOLATIONS2 violations"
echo ""
echo "  Note: These anomalies require SERIALIZABLE isolation to prevent"
echo ""
