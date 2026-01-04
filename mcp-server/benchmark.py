#!/usr/bin/env python3
"""
MCP IronBase Server Benchmark (with Connection Pooling)

Usage: ./benchmark.py [host] [iterations]

This uses HTTP keep-alive connections for accurate server performance measurement.
The old curl-based benchmark measured mostly TLS handshake overhead, not server speed.
"""

import sys
import time
import urllib3
import requests
from requests.adapters import HTTPAdapter
from urllib3.util.retry import Retry

# Disable SSL warnings for self-signed certs
urllib3.disable_warnings(urllib3.exceptions.InsecureRequestWarning)

# Configuration
HOST = sys.argv[1] if len(sys.argv) > 1 else "https://127.0.0.1:8080"
ITERATIONS = int(sys.argv[2]) if len(sys.argv) > 2 else 100

# Create session with connection pooling
session = requests.Session()
adapter = HTTPAdapter(
    pool_connections=1,
    pool_maxsize=1,
    max_retries=Retry(total=0)
)
session.mount("https://", adapter)
session.verify = False  # Self-signed cert

def mcp_call(payload: dict) -> dict:
    """Make an MCP call using the persistent session."""
    response = session.post(
        f"{HOST}/mcp",
        json=payload,
        headers={"Content-Type": "application/json"}
    )
    return response.json()

def benchmark(name: str, payload: dict) -> tuple[float, int]:
    """Run benchmark and return (ms_per_op, ops_per_sec)."""
    print(f"Testing {name}... ", end="", flush=True)

    start = time.perf_counter()
    for _ in range(ITERATIONS):
        mcp_call(payload)
    elapsed = time.perf_counter() - start

    ms_per_op = (elapsed * 1000) / ITERATIONS
    ops_per_sec = int(ITERATIONS / elapsed)

    print(f"{ms_per_op:.2f} ms/op  ({ops_per_sec} ops/sec)")
    return ms_per_op, ops_per_sec

def main():
    print("=" * 50)
    print("  MCP IronBase Server Benchmark (Keep-Alive)")
    print("=" * 50)
    print(f"Host: {HOST}")
    print(f"Iterations: {ITERATIONS}")
    print()

    # Initialize session
    mcp_call({
        "jsonrpc": "2.0",
        "id": 0,
        "method": "initialize",
        "params": {
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": {"name": "benchmark", "version": "1.0"}
        }
    })

    results = []

    # --- Read Operations ---
    print("--- Read Operations ---")

    results.append(("count_documents", benchmark("count_documents (indexed)", {
        "jsonrpc": "2.0", "id": 1,
        "method": "tools/call",
        "params": {"name": "count_documents", "arguments": {"collection": "emails", "query": {}}}
    })))

    results.append(("find_one", benchmark("find_one (by _id)", {
        "jsonrpc": "2.0", "id": 1,
        "method": "tools/call",
        "params": {"name": "find_one", "arguments": {"collection": "emails", "query": {"_id": 1000}}}
    })))

    results.append(("find_10", benchmark("find (limit 10, indexed)", {
        "jsonrpc": "2.0", "id": 1,
        "method": "tools/call",
        "params": {"name": "find", "arguments": {
            "collection": "emails",
            "query": {"from.email": "petitan@petitan.hu"},
            "limit": 10
        }}
    })))

    results.append(("find_100", benchmark("find (limit 100)", {
        "jsonrpc": "2.0", "id": 1,
        "method": "tools/call",
        "params": {"name": "find", "arguments": {
            "collection": "emails",
            "query": {},
            "limit": 100,
            "projection": {"_id": 1, "subject": 1}
        }}
    })))

    print()
    print("--- Aggregation Operations ---")

    results.append(("agg_count", benchmark("aggregate (count only)", {
        "jsonrpc": "2.0", "id": 1,
        "method": "tools/call",
        "params": {"name": "aggregate", "arguments": {
            "collection": "emails",
            "pipeline": [{"$group": {"_id": None, "total": {"$sum": 1}}}]
        }}
    })))

    results.append(("agg_group", benchmark("aggregate (group by field, limit 10)", {
        "jsonrpc": "2.0", "id": 1,
        "method": "tools/call",
        "params": {"name": "aggregate", "arguments": {
            "collection": "emails",
            "pipeline": [
                {"$group": {"_id": "$from.email", "count": {"$sum": 1}}},
                {"$sort": {"count": -1}},
                {"$limit": 10}
            ]
        }}
    })))

    print()
    print("--- Index Operations ---")

    results.append(("index_list", benchmark("index_list", {
        "jsonrpc": "2.0", "id": 1,
        "method": "tools/call",
        "params": {"name": "index_list", "arguments": {"collection": "emails"}}
    })))

    results.append(("explain", benchmark("explain", {
        "jsonrpc": "2.0", "id": 1,
        "method": "tools/call",
        "params": {"name": "explain", "arguments": {
            "collection": "emails",
            "query": {"from.email": "test@test.com"}
        }}
    })))

    print()
    print("--- Write Operations (test collection) ---")

    results.append(("insert_one", benchmark("insert_one", {
        "jsonrpc": "2.0", "id": 1,
        "method": "tools/call",
        "params": {"name": "insert_one", "arguments": {
            "collection": "_benchmark_test",
            "document": {"name": "test", "value": 123, "timestamp": "2024-01-01"}
        }}
    })))

    results.append(("update_one", benchmark("update_one", {
        "jsonrpc": "2.0", "id": 1,
        "method": "tools/call",
        "params": {"name": "update_one", "arguments": {
            "collection": "_benchmark_test",
            "query": {"name": "test"},
            "update": {"$set": {"value": 456}}
        }}
    })))

    results.append(("delete_one", benchmark("delete_one", {
        "jsonrpc": "2.0", "id": 1,
        "method": "tools/call",
        "params": {"name": "delete_one", "arguments": {
            "collection": "_benchmark_test",
            "query": {"name": "test"}
        }}
    })))

    # Cleanup
    mcp_call({
        "jsonrpc": "2.0", "id": 99,
        "method": "tools/call",
        "params": {"name": "drop_collection", "arguments": {"collection": "_benchmark_test"}}
    })

    # Summary
    print()
    print("=" * 50)
    print("  Summary")
    print("=" * 50)

    total_ops = sum(r[1][1] for r in results)
    avg_ops = total_ops // len(results)
    fastest = max(results, key=lambda r: r[1][1])
    slowest = min(results, key=lambda r: r[1][1])

    print(f"Average:  {avg_ops} ops/sec")
    print(f"Fastest:  {fastest[0]} ({fastest[1][1]} ops/sec)")
    print(f"Slowest:  {slowest[0]} ({slowest[1][1]} ops/sec)")
    print()
    print("=" * 50)
    print("  Benchmark Complete")
    print("=" * 50)

if __name__ == "__main__":
    main()
