#!/bin/bash
# Test STDIO mode

rm -f test_stdio2.mlite test_stdio2.wal
export IRONBASE_PATH="test_stdio2.mlite"

echo "=============================================="
echo "=== STDIO MODE TESZT ==="
echo "=============================================="

echo ""
echo "1. Initialize:"
printf '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}\n' | ./target/release/mcp-ironbase-server --stdio 2>/dev/null

echo ""
echo "2. Tools list:"
printf '{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}\n' | ./target/release/mcp-ironbase-server --stdio 2>/dev/null | python3 -c "import sys,json; d=json.load(sys.stdin); print(f'{len(d[\"result\"][\"tools\"])} tools')"

echo ""
echo "3. Prompts list:"
printf '{"jsonrpc":"2.0","id":3,"method":"prompts/list","params":{}}\n' | ./target/release/mcp-ironbase-server --stdio 2>/dev/null | python3 -c "import sys,json; d=json.load(sys.stdin); print(f'{len(d[\"result\"][\"prompts\"])} prompts')"

echo ""
echo "4. Insert document:"
printf '{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"insert_one","arguments":{"collection":"test","document":{"name":"STDIO Test"}}}}\n' | ./target/release/mcp-ironbase-server --stdio 2>/dev/null

echo ""
echo "5. Find document:"
printf '{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"find","arguments":{"collection":"test","query":{}}}}\n' | ./target/release/mcp-ironbase-server --stdio 2>/dev/null

echo ""
echo "=== STDIO MODE: OK ==="
