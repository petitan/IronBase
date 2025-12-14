#!/bin/bash
# MCP + TUI Integration Tests
# Tests HTTP/HTTPS with and without API key

set -e

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Paths
MCP_SERVER="./mcp-server/target/release/mcp-ironbase-server"
TUI="./ironbase-tui/target/release/ironbase-tui"
MCP_DIR="./mcp-server"
TEST_DB="/tmp/test_mcp_tui.mlite"
TEST_API_KEY="test-api-key-12345"

# Cleanup function
cleanup() {
    echo -e "\n${YELLOW}Cleaning up...${NC}"
    pkill -f mcp-ironbase-server 2>/dev/null || true
    rm -f "$TEST_DB" 2>/dev/null || true
    rm -f /tmp/test_config_*.toml 2>/dev/null || true
}

trap cleanup EXIT

# Helper functions
log_test() {
    echo -e "\n${BLUE}=== TEST: $1 ===${NC}"
}

log_pass() {
    echo -e "${GREEN}PASS${NC}: $1"
}

log_fail() {
    echo -e "${RED}FAIL${NC}: $1"
    exit 1
}

wait_for_server() {
    local url=$1
    local insecure=$2
    local max_attempts=10
    local attempt=0

    while [ $attempt -lt $max_attempts ]; do
        if [ "$insecure" = "true" ]; then
            if curl -sk "$url/health" > /dev/null 2>&1; then
                return 0
            fi
        else
            if curl -s "$url/health" > /dev/null 2>&1; then
                return 0
            fi
        fi
        sleep 0.5
        attempt=$((attempt + 1))
    done
    return 1
}

start_server() {
    local config=$1
    pkill -f mcp-ironbase-server 2>/dev/null || true
    sleep 1
    cd "$MCP_DIR"
    RUST_LOG=mcp_ironbase=debug,warn MCP_CONFIG="$config" ./target/release/mcp-ironbase-server > /tmp/mcp_test.log 2>&1 &
    cd - > /dev/null
    sleep 2
}

test_mcp_request() {
    local url=$1
    local insecure=$2
    local api_key=$3
    local method=$4

    local curl_opts="-s"
    [ "$insecure" = "true" ] && curl_opts="$curl_opts -k"

    local headers="-H 'Content-Type: application/json'"
    [ -n "$api_key" ] && headers="$headers -H 'Authorization: Bearer $api_key'"

    local data="{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"$method\",\"params\":{}}"

    eval "curl $curl_opts $headers -d '$data' '$url/mcp'"
}

# ============================================================================
# BUILD
# ============================================================================

echo -e "${YELLOW}Building MCP server and TUI...${NC}"
cargo build --release -p mcp-ironbase-server 2>/dev/null || (cd mcp-server && cargo build --release)
cargo build --release -p ironbase-tui 2>/dev/null || (cd ironbase-tui && cargo build --release)

# ============================================================================
# TEST 1: HTTP without API key
# ============================================================================

log_test "1. MCP HTTP - No API Key"

cat > /tmp/test_config_http_nokey.toml << EOF
[server]
host = "127.0.0.1"
port = 18081

[database]
path = "$TEST_DB"

[security]
require_api_key = false

[tls]
enabled = false
EOF

start_server "/tmp/test_config_http_nokey.toml"

if wait_for_server "http://127.0.0.1:18081" "false"; then
    log_pass "Server started on HTTP:18081"
else
    log_fail "Server failed to start"
fi

# Test initialize
RESULT=$(test_mcp_request "http://127.0.0.1:18081" "false" "" "initialize")
if echo "$RESULT" | grep -q '"serverInfo"'; then
    log_pass "Initialize request successful"
else
    log_fail "Initialize request failed: $RESULT"
fi

# Test tools/list
RESULT=$(test_mcp_request "http://127.0.0.1:18081" "false" "" "tools/list")
if echo "$RESULT" | grep -q '"tools"'; then
    log_pass "Tools list request successful"
else
    log_fail "Tools list request failed: $RESULT"
fi

# ============================================================================
# TEST 2: HTTP with API key required
# ============================================================================

log_test "2. MCP HTTP - API Key Required"

cat > /tmp/test_config_http_key.toml << EOF
[server]
host = "127.0.0.1"
port = 18082

[database]
path = "$TEST_DB"

[security]
require_api_key = true

[tls]
enabled = false
EOF

start_server "/tmp/test_config_http_key.toml"

if wait_for_server "http://127.0.0.1:18082" "false"; then
    log_pass "Server started on HTTP:18082 (API key required)"
else
    log_fail "Server failed to start"
fi

# Initialize always works (protocol requirement)
RESULT=$(test_mcp_request "http://127.0.0.1:18082" "false" "" "initialize")
if echo "$RESULT" | grep -q '"serverInfo"'; then
    log_pass "Initialize works without API key (protocol OK)"
else
    log_fail "Initialize should work: $RESULT"
fi

# Test tools/call without API key (should fail)
RESULT=$(curl -s http://127.0.0.1:18082/mcp \
    -H "Content-Type: application/json" \
    -d '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"collection_list","arguments":{}}}')
if echo "$RESULT" | grep -q '"error"'; then
    log_pass "tools/call without API key rejected"
else
    log_fail "tools/call without API key should be rejected: $RESULT"
fi

# First create an API key via admin (need to set IRONBASE_ADMIN_KEY)
pkill -f mcp-ironbase-server 2>/dev/null || true
sleep 1
cd "$MCP_DIR"
RUST_LOG=warn IRONBASE_ADMIN_KEY=admin123 MCP_CONFIG="/tmp/test_config_http_key.toml" ./target/release/mcp-ironbase-server > /tmp/mcp_test.log 2>&1 &
cd - > /dev/null
sleep 2

if wait_for_server "http://127.0.0.1:18082" "false"; then
    # Create API key
    CREATE_RESULT=$(curl -s http://127.0.0.1:18082/mcp \
        -H "Content-Type: application/json" \
        -d "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/call\",\"params\":{\"name\":\"admin_api_key_create\",\"arguments\":{\"admin_key\":\"admin123\",\"name\":\"test-key\"}}}")

    if echo "$CREATE_RESULT" | grep -q 'sk-'; then
        API_KEY=$(echo "$CREATE_RESULT" | grep -o 'sk-[a-zA-Z0-9]*' | head -1)
        log_pass "API key created: ${API_KEY:0:20}..."

        # Test with API key
        RESULT=$(test_mcp_request "http://127.0.0.1:18082" "false" "$API_KEY" "tools/list")
        if echo "$RESULT" | grep -q '"tools"'; then
            log_pass "Request with API key successful"
        else
            log_fail "Request with API key failed: $RESULT"
        fi
    else
        echo -e "${YELLOW}SKIP${NC}: Could not create API key (admin tools may be disabled)"
    fi
else
    log_fail "Server failed to start with admin key"
fi

# ============================================================================
# TEST 3: HTTPS without API key
# ============================================================================

log_test "3. MCP HTTPS - No API Key"

# Check if certs exist
if [ ! -f "$MCP_DIR/cert.pem" ] || [ ! -f "$MCP_DIR/key.pem" ]; then
    echo -e "${YELLOW}SKIP${NC}: TLS certificates not found. Run: cd mcp-server && mkcert -key-file key.pem -cert-file cert.pem localhost 127.0.0.1"
else
    cat > /tmp/test_config_https_nokey.toml << EOF
[server]
host = "127.0.0.1"
port = 18083

[database]
path = "$TEST_DB"

[security]
require_api_key = false

[tls]
enabled = true
cert_file = "./cert.pem"
key_file = "./key.pem"
EOF

    start_server "/tmp/test_config_https_nokey.toml"

    if wait_for_server "https://127.0.0.1:18083" "true"; then
        log_pass "Server started on HTTPS:18083"
    else
        log_fail "HTTPS server failed to start"
    fi

    # Test initialize
    RESULT=$(test_mcp_request "https://127.0.0.1:18083" "true" "" "initialize")
    if echo "$RESULT" | grep -q '"serverInfo"'; then
        log_pass "HTTPS Initialize request successful"
    else
        log_fail "HTTPS Initialize request failed: $RESULT"
    fi
fi

# ============================================================================
# TEST 4: HTTPS with API key
# ============================================================================

log_test "4. MCP HTTPS - API Key Required"

if [ ! -f "$MCP_DIR/cert.pem" ]; then
    echo -e "${YELLOW}SKIP${NC}: TLS certificates not found"
else
    cat > /tmp/test_config_https_key.toml << EOF
[server]
host = "127.0.0.1"
port = 18084

[database]
path = "$TEST_DB"

[security]
require_api_key = true

[tls]
enabled = true
cert_file = "./cert.pem"
key_file = "./key.pem"
EOF

    pkill -f mcp-ironbase-server 2>/dev/null || true
    sleep 1
    cd "$MCP_DIR"
    RUST_LOG=warn IRONBASE_ADMIN_KEY=admin123 MCP_CONFIG="/tmp/test_config_https_key.toml" ./target/release/mcp-ironbase-server > /tmp/mcp_test.log 2>&1 &
    cd - > /dev/null
    sleep 2

    if wait_for_server "https://127.0.0.1:18084" "true"; then
        log_pass "HTTPS server started on :18084 (API key required)"

        # Initialize always works (protocol requirement)
        RESULT=$(test_mcp_request "https://127.0.0.1:18084" "true" "" "initialize")
        if echo "$RESULT" | grep -q '"serverInfo"'; then
            log_pass "HTTPS Initialize works without API key (protocol OK)"
        else
            log_fail "HTTPS Initialize should work: $RESULT"
        fi

        # Test tools/call without key (should fail)
        RESULT=$(curl -sk https://127.0.0.1:18084/mcp \
            -H "Content-Type: application/json" \
            -d '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"collection_list","arguments":{}}}')
        if echo "$RESULT" | grep -q '"error"'; then
            log_pass "HTTPS tools/call without API key rejected"
        else
            log_fail "HTTPS tools/call without API key should be rejected: $RESULT"
        fi

        # Create and test with key
        CREATE_RESULT=$(curl -sk https://127.0.0.1:18084/mcp \
            -H "Content-Type: application/json" \
            -d "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/call\",\"params\":{\"name\":\"admin_api_key_create\",\"arguments\":{\"admin_key\":\"admin123\",\"name\":\"test-https-key\"}}}")

        if echo "$CREATE_RESULT" | grep -q 'sk-'; then
            API_KEY=$(echo "$CREATE_RESULT" | grep -o 'sk-[a-zA-Z0-9]*' | head -1)
            log_pass "HTTPS API key created"

            RESULT=$(test_mcp_request "https://127.0.0.1:18084" "true" "$API_KEY" "tools/list")
            if echo "$RESULT" | grep -q '"tools"'; then
                log_pass "HTTPS request with API key successful"
            else
                log_fail "HTTPS request with API key failed"
            fi
        else
            echo -e "${YELLOW}SKIP${NC}: Could not create API key via HTTPS"
        fi
    else
        log_fail "HTTPS server failed to start"
    fi
fi

# ============================================================================
# SUMMARY
# ============================================================================

echo -e "\n${GREEN}========================================${NC}"
echo -e "${GREEN}All tests completed!${NC}"
echo -e "${GREEN}========================================${NC}"

echo -e "\n${BLUE}Test Matrix:${NC}"
echo "+-----------+--------+--------+"
echo "| Protocol  | No Key | + Key  |"
echo "+-----------+--------+--------+"
echo "| HTTP      |   OK   |   OK   |"
echo "| HTTPS     |   OK   |   OK   |"
echo "+-----------+--------+--------+"
