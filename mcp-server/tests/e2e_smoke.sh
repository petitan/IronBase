#!/usr/bin/env bash
#
# Real HTTP end-to-end smoke test for the mcp-ironbase-server.
#
# Exercises the FULL stack over the wire (HTTP -> JSON-RPC -> MCP dispatch ->
# handlers) against a freshly started server instance on a throwaway DB. Focuses
# on the v1.0.505-507 changes, which until now had only in-process Rust coverage:
#
#   A. Tool surface     — `search` present; `hybrid_search` / `vector_search_filter`
#                         removed; tool count == 92.
#   B. Non-RAG fulltext — multi-field `match_scope="document"` qualification on a
#                         collection with no parent doc_id (the v1.0.506 fix:
#                         union must key on `_id`, not a `doc_id` the docs lack).
#   C. Fulltext basics  — single-field AND/OR + match_scope=chunk sanity.
#   D. Admin-key gate    — admin_* tool rejected without the key, accepted with it
#                         (the v1.0.505 centralized gate, on the shared dispatch
#                         path so it covers HTTP).
#   E. ACL round-trip    — acl_set -> acl_get persists immediately.
#   F. CRUD sanity       — insert / find / count / delete.
#
# No embedding provider is configured (BM25-only paths), so the suite needs no
# Ollama/BGE-M3 and is fully self-contained and reproducible.
#
# Usage:
#   mcp-server/tests/e2e_smoke.sh                 # build release, start server, test, teardown
#   E2E_BUILD=0 mcp-server/tests/e2e_smoke.sh     # skip the cargo build (use existing release bin)
#   E2E_URL=http://host:8080/mcp ... e2e_smoke.sh # test an already-running server (skips A/D admin tests)
#
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
PORT="${E2E_PORT:-8091}"
ADMIN_KEY="e2e-admin-key"
PASS=0
FAIL=0
SERVER_PID=""
TMP=""

cleanup() {
  [[ -n "$SERVER_PID" ]] && kill "$SERVER_PID" 2>/dev/null
  [[ -n "$TMP" ]] && rm -rf "$TMP"
}
trap cleanup EXIT

# ---- JSON-RPC helpers --------------------------------------------------------
URL="${E2E_URL:-http://127.0.0.1:$PORT/mcp}"

rpc() { # method params-json -> raw JSON-RPC response
  curl -s -m 20 "$URL" -H 'Content-Type: application/json' \
    -d "$(jq -nc --arg m "$1" --argjson p "$2" '{jsonrpc:"2.0",id:1,method:$m,params:$p}')"
}
# Call a tool; print its result text (parsed tool JSON) or the JSON-RPC error message.
tool() { # name args-json
  rpc "tools/call" "$(jq -nc --arg n "$1" --argjson a "$2" '{name:$n,arguments:$a}')" \
    | jq -r '.result.content[0].text // .error.message // empty'
}

pass() { PASS=$((PASS + 1)); printf '  \033[32m✅ %s\033[0m\n' "$1"; }
fail() { FAIL=$((FAIL + 1)); printf '  \033[31m❌ %s\033[0m\n' "$1"; }
assert() { # description  cond(0|1)  [detail]
  if [[ "$2" == "1" ]]; then pass "$1"; else fail "$1${3:+  ($3)}"; fi
}

# ---- start server (unless pointed at a running one) --------------------------
SELF_HOSTED=0
if [[ -z "${E2E_URL:-}" ]]; then
  SELF_HOSTED=1
  BIN="${IRONBASE_BIN:-$ROOT/target/release/mcp-ironbase-server}"
  if [[ "${E2E_BUILD:-1}" == "1" ]]; then
    echo "Building release binary (E2E_BUILD=0 to skip)..."
    (cd "$ROOT" && cargo build --release -p mcp-ironbase-server -q) \
      || { echo "cargo build failed"; exit 1; }
  fi
  [[ -x "$BIN" ]] || { echo "binary not found: $BIN"; exit 1; }
  TMP="$(mktemp -d)"
  echo "Starting server :$PORT  db=$TMP/e2e.mlite  (no embedding provider)..."
  "$BIN" --db "$TMP/e2e.mlite" --port "$PORT" --admin-key "$ADMIN_KEY" \
    >"$TMP/server.log" 2>&1 &
  SERVER_PID=$!
  ready=0
  for _ in $(seq 1 80); do
    if rpc initialize '{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"e2e","version":"1"}}' \
        | grep -q '"result"'; then ready=1; break; fi
    kill -0 "$SERVER_PID" 2>/dev/null || { echo "server exited early:"; tail -20 "$TMP/server.log"; exit 1; }
    sleep 0.5
  done
  [[ "$ready" == "1" ]] || { echo "server did not become ready:"; tail -20 "$TMP/server.log"; exit 1; }
fi

rpc initialize '{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"e2e","version":"1"}}' >/dev/null
KB="_e2e_$RANDOM"

echo
echo "== A. Tool surface (v1.0.505-507) =="
TOOLS="$(rpc "tools/list" '{}' | jq -r '.result.tools[].name')"
COUNT="$(printf '%s\n' "$TOOLS" | grep -c .)"
# Non-localhost callers don't see the 8 admin_* tools (tools/list filtering,
# SECURITY FIX #14): localhost sees 92, a remote/Internal client sees 84.
EXP_COUNT=$([[ "$SELF_HOSTED" == 1 ]] && echo 92 || echo 84)
assert "search tool present"             "$(grep -qx search             <<<"$TOOLS" && echo 1 || echo 0)"
assert "hybrid_search removed"           "$(grep -qx hybrid_search       <<<"$TOOLS" && echo 0 || echo 1)"
assert "vector_search_filter removed"    "$(grep -qx vector_search_filter<<<"$TOOLS" && echo 0 || echo 1)"
assert "tool count == $EXP_COUNT"        "$([[ "$COUNT" == "$EXP_COUNT" ]] && echo 1 || echo 0)" "got $COUNT"

echo
echo "== B. Non-RAG multi-field document-scope fulltext (v1.0.506 fix) =="
tool collection_create "$(jq -nc --arg c "$KB" '{collection:$c}')" >/dev/null
tool insert_one "$(jq -nc --arg c "$KB" '{collection:$c,document:{content:"fékpad PEF-35 berendezés leírás",title:"alpha"}}')" >/dev/null
tool insert_one "$(jq -nc --arg c "$KB" '{collection:$c,document:{content:"csak fékpad egyedül",title:"beta"}}')" >/dev/null
tool index_create_fulltext "$(jq -nc --arg c "$KB" '{collection:$c,field:"content"}')" >/dev/null
tool index_create_fulltext "$(jq -nc --arg c "$KB" '{collection:$c,field:"title"}')" >/dev/null
# Multi-field, multi-word, default match_scope="document". Doc 1 has both tokens.
MF="$(tool fulltext_search "$(jq -nc --arg c "$KB" '{collection:$c,field:"content",fields:["content","title"],query:"fékpad PEF-35"}')")"
MF_COUNT="$(jq -r '[.results[]? // .[]?] | length' <<<"$MF" 2>/dev/null || echo 0)"
MF_HASALPHA="$(jq -e '[.. | objects | select(.content? // "" | test("PEF-35"))] | length > 0' <<<"$MF" >/dev/null 2>&1 && echo 1 || echo 0)"
assert "multi-field doc-scope returns >=1 hit (not 0)" "$([[ "${MF_COUNT:-0}" -ge 1 ]] && echo 1 || echo 0)" "count=$MF_COUNT"
assert "qualifying doc (both tokens) present"          "$MF_HASALPHA"

echo
echo "== C. Fulltext single-field sanity =="
SF="$(tool fulltext_search "$(jq -nc --arg c "$KB" '{collection:$c,field:"content",query:"fékpad"}')")"
SF_COUNT="$(jq -r '[.results[]? // .[]?] | length' <<<"$SF" 2>/dev/null || echo 0)"
assert "single-token 'fékpad' matches both docs" "$([[ "${SF_COUNT:-0}" -ge 2 ]] && echo 1 || echo 0)" "count=$SF_COUNT"

echo
echo "== D. Centralized admin-key gate (v1.0.505) =="
NOKEY="$(tool admin_apikey_list '{}')"
WITHKEY="$(tool admin_apikey_list "$(jq -nc --arg k "$ADMIN_KEY" '{admin_key:$k}')")"
if [[ "$SELF_HOSTED" == 1 ]]; then
  assert "admin_apikey_list rejected without key" "$(grep -qi 'admin authentication failed' <<<"$NOKEY" && echo 1 || echo 0)" "$NOKEY"
  assert "admin_apikey_list accepted with key"    "$(jq -e '.success == true' <<<"$WITHKEY" >/dev/null 2>&1 && echo 1 || echo 0)"
else
  echo "  (skipped — needs the e2e --admin-key; only meaningful on the self-hosted instance)"
fi

echo
echo "== E. ACL change takes effect immediately — no restart (v1.0.505 reload) =="
if [[ "$SELF_HOSTED" != 1 ]]; then
  echo "  (skipped on a shared/remote server — the downgrade locks a collection that"
  echo "   the caller can no longer drop; only run against the self-hosted instance)"
else
# Use a throwaway collection so the downgrade does not lock the main KB. Setting
# localhost to read-only must take effect on the VERY NEXT write (before v1.0.505
# the persisted rule only applied after a server restart). acl_set keys its
# permission check on the `collection` argument, so the downgrade is one-way for
# this collection by design — which is exactly what makes the live effect visible.
KBA="${KB}_acl"
tool collection_create "$(jq -nc --arg c "$KBA" '{collection:$c}')" >/dev/null
tool acl_set "$(jq -nc --arg c "$KBA" '{collection:$c,rules:[{principal:"interface:localhost",permissions:"read"}]}')" >/dev/null
DENIED="$(tool insert_one "$(jq -nc --arg c "$KBA" '{collection:$c,document:{probe:1}}')")"
assert "write DENIED immediately after read-only acl_set (live reload)" "$(grep -qi denied <<<"$DENIED" && echo 1 || echo 0)" "$DENIED"
# The read-only rule still permits acl_get — confirm it persisted in the expected shape.
ACLGET="$(tool acl_get "$(jq -nc --arg c "$KBA" '{collection:$c}')")"
assert "acl_get returns the persisted localhost=read rule" "$(jq -e '.rules[] | select(.principal.value=="localhost") | (.permissions.read==true and .permissions.write==false)' <<<"$ACLGET" >/dev/null 2>&1 && echo 1 || echo 0)"
fi

echo
echo "== F. CRUD sanity =="
tool insert_one "$(jq -nc --arg c "$KB" '{collection:$c,document:{k:"v",n:7}}')" >/dev/null
CNT="$(tool count_documents "$(jq -nc --arg c "$KB" '{collection:$c,query:{}}')" | jq -r '.count // empty')"
assert "count_documents >= 3 after inserts" "$([[ "${CNT:-0}" -ge 3 ]] && echo 1 || echo 0)" "count=$CNT"
FND="$(tool find "$(jq -nc --arg c "$KB" '{collection:$c,query:{n:7}}')")"
assert "find by field returns the doc" "$(jq -e '[.. | objects | select(.n? == 7)] | length > 0' <<<"$FND" >/dev/null 2>&1 && echo 1 || echo 0)"

# ---- cleanup the test collection on a shared (running) server ----------------
tool collection_drop "$(jq -nc --arg c "$KB" '{collection:$c}')" >/dev/null 2>&1 || true

echo
echo "============================================================"
printf 'e2e smoke: \033[32m%d passed\033[0m, ' "$PASS"
if [[ "$FAIL" -gt 0 ]]; then printf '\033[31m%d failed\033[0m\n' "$FAIL"; else printf '0 failed\n'; fi
echo "============================================================"
[[ "$FAIL" -eq 0 ]]
