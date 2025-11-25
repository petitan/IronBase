#!/bin/bash
killall -9 mcp-docjl-server 2>/dev/null
sleep 1
RUST_LOG=debug DOCJL_CONFIG=config.toml ./target/release/mcp-docjl-server 2>&1 | tee /tmp/insert_debug.log &
sleep 4

python3 <<'EOF'
import sys
sys.path.insert(0, 'examples')
from python_client import MCPDocJLClient

client = MCPDocJLClient()
try:
    result = client.insert_block(
        document_id='mk_manual_v1',
        block={'type': 'paragraph', 'content': [{'type': 'text', 'content': 'TEST'}]},
        position='end'
    )
    print(f'SUCCESS: {result}')
except Exception as e:
    print(f'FAILED: {e}')
EOF

sleep 2
echo ""
echo "=== Error messages from server log ==="
grep -i "error\|panic\|failed" /tmp/insert_debug.log | tail -20
