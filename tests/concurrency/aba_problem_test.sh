#!/bin/bash
# ABA Problem Test
# Value changes A→B→A but observer thinks nothing changed
MCP_URL="http://127.0.0.1:8080/mcp"

mcp() {
    curl -s -X POST "$MCP_URL" -H "Content-Type: application/json" -d "$1"
}

get_val() {
    mcp '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"find_one","arguments":{"collection":"aba_test","filter":{"key":"data"}}}}' | \
    python3 -c "import sys,json; d=json.load(sys.stdin); doc=json.loads(d['result']['content'][0]['text'])['document']; print(doc.get('value', 'null'))" 2>/dev/null
}

echo "╔══════════════════════════════════════════════════════════════╗"
echo "║                    ABA PROBLEM TEST                          ║"
echo "║  Value changes A→B→A but observer thinks nothing changed    ║"
echo "╚══════════════════════════════════════════════════════════════╝"
echo ""

#############################################
# TEST 1: CLASSIC ABA - Compare-And-Swap
#############################################
echo "┌─────────────────────────────────────────────────────────────┐"
echo "│  TEST 1: ABA with Compare-And-Swap pattern                 │"
echo "│  Thread1: if value==A then update                          │"
echo "│  Thread2: A→B→A (behind Thread1's back)                    │"
echo "└─────────────────────────────────────────────────────────────┘"
echo ""

ABA_UNDETECTED=0
ROUNDS=20

for round in $(seq 1 $ROUNDS); do
    # Setup: value = 100
    mcp '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"delete_many","arguments":{"collection":"aba_test","filter":{}}}}' > /dev/null
    mcp '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"insert_one","arguments":{"collection":"aba_test","document":{"key":"data","value":100}}}}' > /dev/null

    # Thread1: Slow CAS - reads, sleeps, then updates if still same
    thread1_cas() {
        # 1. Read original value
        local original=$(get_val)

        # 2. Sleep (simulate slow processing)
        sleep 0.1

        # 3. CAS: update only if value unchanged
        local result=$(mcp '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"update_one","arguments":{"collection":"aba_test","filter":{"key":"data","value":'$original'},"update":{"$set":{"value":999}}}}}')

        if echo "$result" | grep -q '"modified_count"[^0-9]*1'; then
            echo "CAS_SUCCESS"
        else
            echo "CAS_FAILED"
        fi
    }

    # Thread2: Quick A→B→A flip
    thread2_flip() {
        # A→B
        mcp '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"update_one","arguments":{"collection":"aba_test","filter":{"key":"data"},"update":{"$set":{"value":200}}}}}' > /dev/null

        # B→A (back to original)
        mcp '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"update_one","arguments":{"collection":"aba_test","filter":{"key":"data"},"update":{"$set":{"value":100}}}}}' > /dev/null

        echo "FLIP_DONE"
    }

    # Run concurrently: Thread1 starts, Thread2 flips during Thread1's sleep
    RESULT1=$(thread1_cas) &
    P1=$!
    sleep 0.02  # Let Thread1 read first
    RESULT2=$(thread2_flip)
    wait $P1

    # Check: Did Thread1's CAS succeed despite the flip?
    FINAL=$(get_val)

    if [[ "$FINAL" == "999" ]]; then
        ((ABA_UNDETECTED++))
        echo "  👻 Round $round: ABA undetected! CAS succeeded after A→B→A flip"
    fi
done

echo ""
echo "  Rounds: $ROUNDS"
echo "  ABA undetected: $ABA_UNDETECTED"
echo ""

if [[ "$ABA_UNDETECTED" -gt 0 ]]; then
    echo "  ⚠️ ABA Problem detected $ABA_UNDETECTED times"
    echo "  (CAS succeeded even though value changed A→B→A)"
else
    echo "  ✅ No ABA detected (timing may not have aligned)"
fi

echo ""

#############################################
# TEST 2: ABA with Side Effects
#############################################
echo "┌─────────────────────────────────────────────────────────────┐"
echo "│  TEST 2: ABA with Side Effects (Item Ownership)            │"
echo "│  Item: owner=Alice → Bob → Alice                           │"
echo "│  Problem: Bob's ownership period is invisible              │"
echo "└─────────────────────────────────────────────────────────────┘"
echo ""

# Setup: item owned by Alice, with transfer history
mcp '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"delete_many","arguments":{"collection":"ownership","filter":{}}}}' > /dev/null
mcp '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"insert_one","arguments":{"collection":"ownership","document":{"item":"diamond","owner":"Alice","version":1}}}}' > /dev/null

echo "Initial: owner=Alice, version=1"

# Simulate: Alice→Bob→Alice transfer
mcp '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"update_one","arguments":{"collection":"ownership","filter":{"item":"diamond"},"update":{"$set":{"owner":"Bob"},"$inc":{"version":1}}}}}' > /dev/null
echo "Transfer 1: Alice → Bob (version=2)"

mcp '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"update_one","arguments":{"collection":"ownership","filter":{"item":"diamond"},"update":{"$set":{"owner":"Alice"},"$inc":{"version":1}}}}}' > /dev/null
echo "Transfer 2: Bob → Alice (version=3)"

# Query without version
OWNER_ONLY=$(mcp '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"find_one","arguments":{"collection":"ownership","filter":{"item":"diamond"}}}}' | \
python3 -c "import sys,json; d=json.load(sys.stdin); doc=json.loads(d['result']['content'][0]['text'])['document']; print(f\"owner={doc['owner']}, version={doc['version']}\")" 2>/dev/null)

echo ""
echo "Final state: $OWNER_ONLY"
echo ""

echo "  Without version field:"
echo "    Query {owner: 'Alice'} would match"
echo "    → Cannot detect that Bob owned it briefly!"
echo ""
echo "  With version field:"
echo "    version=3 proves 2 changes happened"
echo "    → ABA is detectable via version check"
echo ""

#############################################
# TEST 3: Solution - Version-based CAS
#############################################
echo "┌─────────────────────────────────────────────────────────────┐"
echo "│  TEST 3: SOLUTION - Version-based Compare-And-Swap         │"
echo "│  CAS uses version field instead of value                   │"
echo "└─────────────────────────────────────────────────────────────┘"
echo ""

SAFE_ROUNDS=20
SAFE_FAILURES=0

for round in $(seq 1 $SAFE_ROUNDS); do
    # Setup with version
    mcp '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"delete_many","arguments":{"collection":"versioned","filter":{}}}}' > /dev/null
    mcp '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"insert_one","arguments":{"collection":"versioned","document":{"key":"data","value":100,"version":1}}}}' > /dev/null

    # Thread1: Version-based CAS
    thread1_versioned() {
        # 1. Read value AND version
        local doc=$(mcp '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"find_one","arguments":{"collection":"versioned","filter":{"key":"data"}}}}')
        local version=$(echo "$doc" | python3 -c "import sys,json; d=json.load(sys.stdin); doc=json.loads(d['result']['content'][0]['text'])['document']; print(doc.get('version', 0))" 2>/dev/null)

        # 2. Sleep
        sleep 0.1

        # 3. CAS with VERSION check (not value!)
        local result=$(mcp '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"update_one","arguments":{"collection":"versioned","filter":{"key":"data","version":'$version'},"update":{"$set":{"value":999},"$inc":{"version":1}}}}}')

        if echo "$result" | grep -q '"modified_count"[^0-9]*1'; then
            echo "VCAS_SUCCESS"
        else
            echo "VCAS_FAILED"
        fi
    }

    # Thread2: A→B→A but version changes!
    thread2_versioned() {
        mcp '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"update_one","arguments":{"collection":"versioned","filter":{"key":"data"},"update":{"$set":{"value":200},"$inc":{"version":1}}}}}' > /dev/null
        mcp '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"update_one","arguments":{"collection":"versioned","filter":{"key":"data"},"update":{"$set":{"value":100},"$inc":{"version":1}}}}}' > /dev/null
    }

    # Run
    RESULT1=$(thread1_versioned) &
    P1=$!
    sleep 0.02
    thread2_versioned
    wait $P1

    # Check: Thread1's CAS should FAIL because version changed
    FINAL=$(mcp '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"find_one","arguments":{"collection":"versioned","filter":{"key":"data"}}}}' | \
    python3 -c "import sys,json; d=json.load(sys.stdin); doc=json.loads(d['result']['content'][0]['text'])['document']; print(doc.get('value', 0))" 2>/dev/null)

    if [[ "$FINAL" == "999" ]]; then
        ((SAFE_FAILURES++))
        echo "  ❌ Round $round: Version CAS failed to detect ABA!"
    fi
done

echo ""
echo "  Rounds: $SAFE_ROUNDS"
echo "  False successes: $SAFE_FAILURES"
echo ""

if [[ "$SAFE_FAILURES" -eq 0 ]]; then
    echo "  ✅ Version-based CAS correctly detects ABA!"
else
    echo "  ❌ Unexpected: Version CAS had $SAFE_FAILURES failures"
fi

echo ""
echo "╔══════════════════════════════════════════════════════════════╗"
echo "║                        SUMMARY                               ║"
echo "╚══════════════════════════════════════════════════════════════╝"
echo ""
echo "  ABA Problem: Value changes A→B→A but looks unchanged"
echo ""
echo "  ❌ Vulnerable: CAS on value only"
echo "     update({value: 100}, {value: 999})  // A→B→A passes!"
echo ""
echo "  ✅ Solution: CAS on version field"
echo "     update({version: 1}, {value: 999, version: 2})"
echo "     // A→B→A changes version 1→2→3, so CAS fails correctly"
echo ""
