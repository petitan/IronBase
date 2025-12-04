#!/bin/bash
# Double Spending Attack Test
# Tests if two concurrent transactions can spend the same money twice
MCP_URL="http://127.0.0.1:8080/mcp"

mcp() {
    curl -s -X POST "$MCP_URL" -H "Content-Type: application/json" -d "$1"
}

get_balance() {
    local user=$1
    mcp '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"find_one","arguments":{"collection":"accounts","filter":{"user":"'$user'"}}}}' | \
    python3 -c "import sys,json; d=json.load(sys.stdin); doc=json.loads(d['result']['content'][0]['text'])['document']; print(doc.get('balance', 0))" 2>/dev/null
}

echo "╔══════════════════════════════════════════════════════════════╗"
echo "║              DOUBLE SPENDING ATTACK TEST                     ║"
echo "╚══════════════════════════════════════════════════════════════╝"
echo ""

#############################################
# TEST 1: VULNERABLE - Read-Then-Write Pattern
#############################################
echo "┌─────────────────────────────────────────────────────────────┐"
echo "│  TEST 1: VULNERABLE PATTERN (Read → Check → Write)         │"
echo "│  Balance: 100, Two concurrent payments of 80 each          │"
echo "└─────────────────────────────────────────────────────────────┘"
echo ""

DOUBLE_SPENDS=0
ROUNDS=20

for round in $(seq 1 $ROUNDS); do
    # Reset: Alice has 100
    mcp '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"delete_many","arguments":{"collection":"accounts","filter":{}}}}' > /dev/null
    mcp '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"insert_one","arguments":{"collection":"accounts","document":{"user":"alice","balance":100}}}}' > /dev/null

    # Vulnerable payment function: READ → CHECK → WRITE
    vulnerable_payment() {
        local amount=$1
        local recipient=$2

        # 1. Read current balance
        local balance=$(get_balance "alice")

        # 2. Check if enough funds
        if [[ "$balance" -ge "$amount" ]]; then
            # 3. Deduct (VULNERABLE: balance may have changed!)
            mcp '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"update_one","arguments":{"collection":"accounts","filter":{"user":"alice"},"update":{"$inc":{"balance":-'$amount'}}}}}' > /dev/null
            echo "PAY_$recipient:SUCCESS"
        else
            echo "PAY_$recipient:INSUFFICIENT"
        fi
    }

    # Two concurrent payments of 80 each
    R1=$(vulnerable_payment 80 "Bob") &
    P1=$!
    R2=$(vulnerable_payment 80 "Charlie") &
    P2=$!
    wait $P1 $P2

    # Check final balance
    FINAL=$(get_balance "alice")

    if [[ "$FINAL" -lt 0 ]]; then
        ((DOUBLE_SPENDS++))
        echo "  💸 Round $round: DOUBLE SPEND! Final balance=$FINAL (paid 160 from 100)"
    fi
done

echo ""
echo "  Rounds: $ROUNDS"
echo "  Double spends: $DOUBLE_SPENDS"
echo ""

if [[ "$DOUBLE_SPENDS" -gt 0 ]]; then
    echo "  ❌ VULNERABLE - $DOUBLE_SPENDS double spending attacks succeeded!"
else
    echo "  ✅ No double spends detected (timing may not have aligned)"
fi

echo ""

#############################################
# TEST 2: SAFE - Conditional Update Pattern
#############################################
echo "┌─────────────────────────────────────────────────────────────┐"
echo "│  TEST 2: SAFE PATTERN (Atomic Conditional Update)          │"
echo "│  update_one({balance: {$gte: 80}}, {$inc: {balance: -80}}) │"
echo "└─────────────────────────────────────────────────────────────┘"
echo ""

SAFE_DOUBLE_SPENDS=0
SAFE_ROUNDS=20

for round in $(seq 1 $SAFE_ROUNDS); do
    # Reset: Alice has 100
    mcp '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"delete_many","arguments":{"collection":"accounts","filter":{}}}}' > /dev/null
    mcp '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"insert_one","arguments":{"collection":"accounts","document":{"user":"alice","balance":100}}}}' > /dev/null

    # Safe payment: Atomic conditional update
    safe_payment() {
        local amount=$1
        local recipient=$2

        # Single atomic operation: only deduct if balance >= amount
        local result=$(mcp '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"update_one","arguments":{"collection":"accounts","filter":{"user":"alice","balance":{"$gte":'$amount'}},"update":{"$inc":{"balance":-'$amount'}}}}}')

        if echo "$result" | grep -q '"modified_count"[^0-9]*1'; then
            echo "SAFE_PAY_$recipient:SUCCESS"
        else
            echo "SAFE_PAY_$recipient:REJECTED"
        fi
    }

    # Two concurrent payments of 80 each
    safe_payment 80 "Bob" &
    P1=$!
    safe_payment 80 "Charlie" &
    P2=$!
    wait $P1 $P2

    # Check final balance
    FINAL=$(get_balance "alice")

    if [[ "$FINAL" -lt 0 ]]; then
        ((SAFE_DOUBLE_SPENDS++))
        echo "  💸 Round $round: DOUBLE SPEND! Final balance=$FINAL"
    elif [[ "$FINAL" -eq 20 ]]; then
        : # Expected: one payment succeeded (100-80=20)
    elif [[ "$FINAL" -eq 100 ]]; then
        : # Both rejected (unlikely with 100 >= 80)
    fi
done

echo ""
echo "  Rounds: $SAFE_ROUNDS"
echo "  Double spends: $SAFE_DOUBLE_SPENDS"
echo ""

if [[ "$SAFE_DOUBLE_SPENDS" -eq 0 ]]; then
    echo "  ✅ SAFE - No double spending possible with conditional updates!"
else
    echo "  ❌ Unexpected: $SAFE_DOUBLE_SPENDS double spends with safe pattern"
fi

echo ""

#############################################
# TEST 3: HIGH CONCURRENCY STRESS TEST
#############################################
echo "┌─────────────────────────────────────────────────────────────┐"
echo "│  TEST 3: STRESS TEST (5 threads × 50 payments = 5000)      │"
echo "│  Balance: 1000, Each payment: 100                          │"
echo "│  Expected: max 10 successful payments (1000/100)           │"
echo "└─────────────────────────────────────────────────────────────┘"
echo ""

# Setup
mcp '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"delete_many","arguments":{"collection":"stress_test","filter":{}}}}' > /dev/null
mcp '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"insert_one","arguments":{"collection":"stress_test","document":{"user":"vault","balance":1000}}}}' > /dev/null

# Counter for successful payments (using temp file for cross-process counting)
SUCCESS_FILE=$(mktemp)
echo "0" > "$SUCCESS_FILE"

stress_payment() {
    local thread_id=$1
    local successes=0

    for i in $(seq 1 50); do
        result=$(mcp '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"update_one","arguments":{"collection":"stress_test","filter":{"user":"vault","balance":{"$gte":100}},"update":{"$inc":{"balance":-100}}}}}')

        if echo "$result" | grep -q '"modified_count"[^0-9]*1'; then
            ((successes++))
        fi
    done

    echo "THREAD_$thread_id:$successes"
}

START=$(date +%s.%N)

# Run 5 threads concurrently
stress_payment 1 &
P1=$!
stress_payment 2 &
P2=$!
stress_payment 3 &
P3=$!
stress_payment 4 &
P4=$!
stress_payment 5 &
P5=$!

# Collect results
wait $P1 $P2 $P3 $P4 $P5

END=$(date +%s.%N)

# Final balance
FINAL_BALANCE=$(mcp '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"find_one","arguments":{"collection":"stress_test","filter":{"user":"vault"}}}}' | \
python3 -c "import sys,json; d=json.load(sys.stdin); doc=json.loads(d['result']['content'][0]['text'])['document']; print(doc.get('balance', 0))" 2>/dev/null)

TOTAL_SPENT=$((1000 - FINAL_BALANCE))
SUCCESSFUL_PAYMENTS=$((TOTAL_SPENT / 100))

echo ""
echo "  Initial balance: 1000"
echo "  Final balance: $FINAL_BALANCE"
echo "  Total spent: $TOTAL_SPENT"
echo "  Successful payments: $SUCCESSFUL_PAYMENTS (expected: 10)"
echo "  Time: $(echo "$END - $START" | bc)s"
echo ""

if [[ "$FINAL_BALANCE" -ge 0 ]]; then
    echo "  ✅ PASS - No overdraft! Conditional updates work correctly."
else
    echo "  ❌ FAIL - Overdraft detected! Balance went negative: $FINAL_BALANCE"
fi

rm -f "$SUCCESS_FILE"

echo ""
echo "╔══════════════════════════════════════════════════════════════╗"
echo "║                        SUMMARY                               ║"
echo "╚══════════════════════════════════════════════════════════════╝"
echo ""
echo "  Vulnerable Pattern (Read→Check→Write):"
echo "    Double spends: $DOUBLE_SPENDS/$ROUNDS"
echo ""
echo "  Safe Pattern (Atomic Conditional Update):"
echo "    Double spends: $SAFE_DOUBLE_SPENDS/$SAFE_ROUNDS"
echo ""
echo "  Stress Test (5 threads × 50 = 250 attempts):"
echo "    Final balance: $FINAL_BALANCE (expected: >= 0)"
echo ""
echo "┌─────────────────────────────────────────────────────────────┐"
echo "│  RECOMMENDATION                                            │"
echo "│  Always use conditional updates for financial operations:  │"
echo "│                                                            │"
echo "│  ❌ BAD:  read balance → if >= amount → update            │"
echo "│  ✅ GOOD: update_one({balance: {\$gte: amount}}, ...)      │"
echo "└─────────────────────────────────────────────────────────────┘"
echo ""
