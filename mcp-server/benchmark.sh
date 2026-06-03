#!/bin/bash
# MCP Server Benchmark Script
# Usage: ./benchmark.sh [host] [iterations]

HOST="${1:-https://127.0.0.1:8080}"
ITERATIONS="${2:-100}"

echo "============================================"
echo "  MCP IronBase Server Benchmark"
echo "============================================"
echo "Host: $HOST"
echo "Iterations: $ITERATIONS"
echo ""

# Initialize session
curl -sk -X POST "$HOST/mcp" \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":0,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"benchmark","version":"1.0"}}}' > /dev/null 2>&1

benchmark() {
    local name="$1"
    local payload="$2"
    local start end elapsed avg

    echo -n "Testing $name... "

    start=$(date +%s%N)
    for i in $(seq 1 $ITERATIONS); do
        curl -sk -X POST "$HOST/mcp" \
          -H "Content-Type: application/json" \
          -d "$payload" > /dev/null 2>&1
    done
    end=$(date +%s%N)

    elapsed=$(( (end - start) / 1000000 ))
    avg=$(echo "scale=2; $elapsed / $ITERATIONS" | bc)
    ops=$(echo "scale=0; 1000 * $ITERATIONS / $elapsed" | bc)

    printf "%.2f ms/op  (%d ops/sec)\n" "$avg" "$ops"
}

echo "--- Read Operations ---"

benchmark "count_documents (indexed)" \
  '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"count","arguments":{"collection":"emails","query":{}}}}'

benchmark "find_one (by _id)" \
  '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"find_one","arguments":{"collection":"emails","query":{"_id":1000}}}}'

benchmark "find (limit 10, indexed)" \
  '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"find","arguments":{"collection":"emails","query":{"from.email":"petitan@petitan.hu"},"limit":10}}}'

benchmark "find (limit 100)" \
  '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"find","arguments":{"collection":"emails","query":{},"limit":100,"projection":{"_id":1,"subject":1}}}}'

echo ""
echo "--- Aggregation Operations ---"

benchmark "aggregate (count only)" \
  '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"aggregate","arguments":{"collection":"emails","pipeline":[{"$group":{"_id":null,"total":{"$sum":1}}}]}}}'

benchmark "aggregate (group by field, limit 10)" \
  '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"aggregate","arguments":{"collection":"emails","pipeline":[{"$group":{"_id":"$from.email","count":{"$sum":1}}},{"$sort":{"count":-1}},{"$limit":10}]}}}'

echo ""
echo "--- Index Operations ---"

benchmark "index_list" \
  '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"index_list","arguments":{"collection":"emails"}}}'

benchmark "explain" \
  '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"explain","arguments":{"collection":"emails","query":{"from.email":"test@test.com"}}}}'

echo ""
echo "--- Write Operations (test collection) ---"

# Create test collection for write benchmarks
benchmark "insert_one" \
  '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"insert_one","arguments":{"collection":"_benchmark_test","document":{"name":"test","value":123,"timestamp":"2024-01-01"}}}}'

benchmark "update_one" \
  '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"update_one","arguments":{"collection":"_benchmark_test","query":{"name":"test"},"update":{"$set":{"value":456}}}}}'

benchmark "delete_one" \
  '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"delete_one","arguments":{"collection":"_benchmark_test","query":{"name":"test"}}}}'

# Cleanup
curl -sk -X POST "$HOST/mcp" \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":99,"method":"tools/call","params":{"name":"drop_collection","arguments":{"collection":"_benchmark_test"}}}' > /dev/null 2>&1

echo ""
echo "============================================"
echo "  Benchmark Complete"
echo "============================================"
