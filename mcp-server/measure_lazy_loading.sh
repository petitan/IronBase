#!/bin/bash
# Lazy Loading Memory Measurement Script
# Measures memory before and after V1 → V2 conversion

set -e

SERVER="./target/release/mcp-ironbase-server"
DATA_FILE="ironbase_data.mlite"
FTIDX_FILES=$(find . -name "*.ftidx" 2>/dev/null | head -5)

echo "=== FulltextIndex Lazy Loading Memory Test ==="
echo ""

# Check if server binary exists
if [ ! -f "$SERVER" ]; then
    echo "ERROR: Server binary not found. Run: cargo build --release"
    exit 1
fi

# Check ftidx files
echo "Found .ftidx files:"
find . -name "*.ftidx" -exec ls -lh {} \; 2>/dev/null || echo "  (none)"
echo ""

# Function to get memory usage
get_memory() {
    local pid=$1
    if [ -n "$pid" ]; then
        ps -o rss= -p $pid 2>/dev/null | awk '{printf "%.1f", $1/1024}'
    else
        echo "0"
    fi
}

# Function to check ftidx version
check_ftidx_version() {
    for f in $(find . -name "*.ftidx" 2>/dev/null); do
        # Read first 12 bytes: 8 magic + 4 version
        VERSION=$(xxd -l 12 "$f" 2>/dev/null | head -1 | awk '{print $6}' | sed 's/^0*//')
        if [ "$VERSION" = "1" ] || [ "$VERSION" = "0100" ]; then
            echo "  $f: V1 (in-memory)"
        elif [ "$VERSION" = "2" ] || [ "$VERSION" = "0200" ]; then
            echo "  $f: V2 (lazy loading)"
        else
            echo "  $f: Unknown version"
        fi
    done
}

echo "=== Phase 1: Check current .ftidx versions ==="
check_ftidx_version
echo ""

echo "=== Phase 2: Start server (V1 load - full memory) ==="
$SERVER &
SERVER_PID=$!
sleep 5

MEM_V1=$(get_memory $SERVER_PID)
echo "Memory after V1 load: ${MEM_V1} MB"
echo "PID: $SERVER_PID"
echo ""

echo "=== Phase 3: Trigger flush (V1 → V2 conversion) ==="
# Call compact to trigger flush
curl -sk -X POST https://127.0.0.1:8080/mcp \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"compact","arguments":{}}}' \
  | jq -r '.result.content[0].text' 2>/dev/null || echo "(compact called)"

sleep 2

MEM_AFTER_FLUSH=$(get_memory $SERVER_PID)
echo "Memory after flush: ${MEM_AFTER_FLUSH} MB"
echo ""

echo "=== Phase 4: Stop server ==="
kill $SERVER_PID 2>/dev/null || true
wait $SERVER_PID 2>/dev/null || true
sleep 2
echo "Server stopped"
echo ""

echo "=== Phase 5: Check .ftidx versions after flush ==="
check_ftidx_version
echo ""

echo "=== Phase 6: Restart server (V2 load - lazy loading) ==="
$SERVER &
SERVER_PID=$!
sleep 5

MEM_V2=$(get_memory $SERVER_PID)
echo "Memory after V2 load: ${MEM_V2} MB"
echo "PID: $SERVER_PID"
echo ""

echo "=== Phase 7: Verify search still works ==="
SEARCH_RESULT=$(curl -sk -X POST https://127.0.0.1:8080/mcp \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"fulltext_search","arguments":{"collection":"emails","field":"body","query":"hello","limit":3}}}' \
  | jq -r '.result.content[0].text' 2>/dev/null)
echo "Search test result: $(echo "$SEARCH_RESULT" | head -c 200)..."
echo ""

echo "=== Phase 8: Cleanup ==="
kill $SERVER_PID 2>/dev/null || true
wait $SERVER_PID 2>/dev/null || true
echo "Server stopped"
echo ""

echo "=========================================="
echo "           RESULTS SUMMARY"
echo "=========================================="
echo ""
echo "Memory V1 (in-memory):     ${MEM_V1} MB"
echo "Memory V2 (lazy loading):  ${MEM_V2} MB"
echo ""
if [ -n "$MEM_V1" ] && [ -n "$MEM_V2" ] && [ "$MEM_V1" != "0" ]; then
    SAVINGS=$(echo "scale=1; $MEM_V1 - $MEM_V2" | bc)
    PERCENT=$(echo "scale=1; ($MEM_V1 - $MEM_V2) / $MEM_V1 * 100" | bc)
    echo "Memory saved:              ${SAVINGS} MB (${PERCENT}%)"
fi
echo "=========================================="
