#!/bin/bash
# Concurrent Insert/Read Test for MCP Server
# Tests that one process can insert while another reads in parallel

set -e

MCP_URL="${MCP_URL:-http://127.0.0.1:8080/mcp}"
COLLECTION="concurrent_test_$(date +%s)"
INSERT_COUNT=100
READ_COUNT=200

echo "=== Concurrent Insert/Read Test ==="
echo "MCP URL: $MCP_URL"
echo "Collection: $COLLECTION"
echo "Inserts: $INSERT_COUNT, Reads: $READ_COUNT"
echo ""

# Initialize MCP session
echo "Initializing MCP session..."
init_result=$(curl -s -X POST "$MCP_URL" \
    -H "Content-Type: application/json" \
    -d '{"jsonrpc":"2.0","id":0,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"concurrent-test","version":"1.0"}}}')

if echo "$init_result" | grep -q '"error"'; then
    echo "Failed to initialize: $init_result"
    exit 1
fi
echo "Initialized successfully"

# Cleanup function
cleanup() {
    echo ""
    echo "=== Cleanup ==="
    curl -s -X POST "$MCP_URL" \
        -H "Content-Type: application/json" \
        -d "{\"jsonrpc\":\"2.0\",\"id\":999,\"method\":\"tools/call\",\"params\":{\"name\":\"collection_drop\",\"arguments\":{\"name\":\"$COLLECTION\"}}}" > /dev/null 2>&1 || true
    echo "Collection dropped"
}
trap cleanup EXIT

# Inserter function (runs in subshell)
inserter() {
    local success=0
    local fail=0
    for i in $(seq 1 $INSERT_COUNT); do
        result=$(curl -s -X POST "$MCP_URL" \
            -H "Content-Type: application/json" \
            -d "{\"jsonrpc\":\"2.0\",\"id\":$i,\"method\":\"tools/call\",\"params\":{\"name\":\"insert_one\",\"arguments\":{\"collection\":\"$COLLECTION\",\"document\":{\"seq\":$i,\"timestamp\":$(date +%s%N),\"data\":\"test_record_$i\"}}}}" 2>&1)
        if echo "$result" | grep -q '"result"'; then
            ((success++)) || true
        else
            ((fail++)) || true
        fi
    done
    echo "INSERTER_DONE:$success:$fail"
}

# Reader function (runs in subshell)
reader() {
    local success=0
    local fail=0
    local max_docs=0
    for i in $(seq 1 $READ_COUNT); do
        result=$(curl -s -X POST "$MCP_URL" \
            -H "Content-Type: application/json" \
            -d "{\"jsonrpc\":\"2.0\",\"id\":$((1000+i)),\"method\":\"tools/call\",\"params\":{\"name\":\"find\",\"arguments\":{\"collection\":\"$COLLECTION\",\"query\":{}}}}" 2>&1)
        if echo "$result" | grep -q '"result"'; then
            ((success++)) || true
            # JSON response has escaped quotes: \"seq\" not "seq"
            docs=$(echo "$result" | grep -o '\\"seq\\"' | wc -l)
            if [ "$docs" -gt "$max_docs" ]; then
                max_docs=$docs
            fi
        else
            ((fail++)) || true
        fi
    done
    echo "READER_DONE:$success:$fail:$max_docs"
}

echo ""
echo "=== Starting concurrent test ==="
echo "Inserter and Reader running in parallel..."
echo ""

# Create temp files for output
INSERTER_OUT=$(mktemp)
READER_OUT=$(mktemp)

# Start both in parallel
inserter > "$INSERTER_OUT" 2>&1 &
INSERTER_PID=$!

reader > "$READER_OUT" 2>&1 &
READER_PID=$!

# Wait for both
wait $INSERTER_PID || true
wait $READER_PID || true

# Get results
INSERTER_RESULT=$(cat "$INSERTER_OUT" | grep "INSERTER_DONE" || echo "INSERTER_DONE:0:0")
READER_RESULT=$(cat "$READER_OUT" | grep "READER_DONE" || echo "READER_DONE:0:0:0")

rm -f "$INSERTER_OUT" "$READER_OUT"

# Parse results
INS_SUCCESS=$(echo "$INSERTER_RESULT" | cut -d: -f2)
INS_FAIL=$(echo "$INSERTER_RESULT" | cut -d: -f3)
READ_SUCCESS=$(echo "$READER_RESULT" | cut -d: -f2)
READ_FAIL=$(echo "$READER_RESULT" | cut -d: -f3)
READ_MAX=$(echo "$READER_RESULT" | cut -d: -f4)

echo "INSERTER: $INS_SUCCESS success, $INS_FAIL fail"
echo "READER: $READ_SUCCESS success, $READ_FAIL fail, max docs seen: $READ_MAX"

echo ""
echo "=== Final verification ==="

# Final count
final_result=$(curl -s -X POST "$MCP_URL" \
    -H "Content-Type: application/json" \
    -d "{\"jsonrpc\":\"2.0\",\"id\":9999,\"method\":\"tools/call\",\"params\":{\"name\":\"find\",\"arguments\":{\"collection\":\"$COLLECTION\",\"query\":{}}}}")

# JSON response has escaped quotes: \"seq\" not "seq"
final_count=$(echo "$final_result" | grep -o '\\"seq\\"' | wc -l)
echo "Final document count: $final_count (expected: $INSERT_COUNT)"

if [ "$final_count" -eq "$INSERT_COUNT" ]; then
    echo ""
    echo "=== TEST PASSED ==="
    exit 0
else
    echo ""
    echo "=== TEST FAILED ==="
    echo "Expected $INSERT_COUNT documents, got $final_count"
    exit 1
fi
