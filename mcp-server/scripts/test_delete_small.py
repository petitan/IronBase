#!/usr/bin/env python3
"""
Test delete_many with a SMALL batch to isolate the hanging issue.
"""

import json
import requests
import urllib3
import time

urllib3.disable_warnings(urllib3.exceptions.InsecureRequestWarning)

MCP_URL = "https://localhost:8080/mcp"
HEADERS = {"Content-Type": "application/json"}
request_id = 0


def call_mcp(method: str, params: dict = None) -> dict:
    global request_id
    request_id += 1
    payload = {"jsonrpc": "2.0", "id": request_id, "method": method, "params": params or {}}
    resp = requests.post(MCP_URL, json=payload, headers=HEADERS, verify=False, timeout=120)
    return resp.json()


def call_tool(name: str, arguments: dict):
    result = call_mcp("tools/call", {"name": name, "arguments": arguments})
    if "result" in result and "content" in result["result"]:
        text = result["result"]["content"][0].get("text", "")
        try:
            return json.loads(text)
        except:
            return text
    if "error" in result:
        raise RuntimeError(f"MCP error: {result['error']}")
    return result


def main():
    call_mcp("initialize", {
        "protocolVersion": "2024-11-05",
        "capabilities": {},
        "clientInfo": {"name": "delete-test", "version": "1.0"}
    })

    print("=" * 60)
    print("Testing delete_many with small batch")
    print("=" * 60)

    # Step 1: Find ONE duplicate message_id
    print("\n[Step 1] Finding one duplicate via aggregation...")
    pipeline = [
        {"$group": {"_id": "$message_id", "count": {"$sum": 1}}},
        {"$match": {"count": {"$gt": 1}}},
        {"$limit": 1}
    ]

    start = time.time()
    result = call_tool("aggregate", {
        "collection": "emails",
        "pipeline": pipeline
    })
    print(f"         Aggregation took {time.time() - start:.2f}s")

    duplicates = result.get("results", [])
    if not duplicates:
        print("No duplicates found!")
        return

    dup = duplicates[0]
    message_id = dup["_id"]
    count = dup["count"]
    print(f"         Found duplicate: {message_id[:50]}... ({count}x)")

    # Step 2: Get _ids for this duplicate
    print("\n[Step 2] Getting _ids for this duplicate...")
    start = time.time()
    result = call_tool("find", {
        "collection": "emails",
        "filter": {"message_id": message_id},
        "projection": {"_id": 1}
    })
    print(f"         Find took {time.time() - start:.2f}s")

    docs = result.get("documents", [])
    ids = [doc["_id"] for doc in docs if isinstance(doc, dict)]
    print(f"         Found {len(ids)} documents")

    if len(ids) <= 1:
        print("Only one document, nothing to delete")
        return

    # Keep first, delete others
    ids_to_delete = sorted(ids)[1:]
    print(f"         Will delete {len(ids_to_delete)} duplicates")

    # Step 3: Delete with small batch
    print("\n[Step 3] Deleting duplicates...")
    print(f"         IDs to delete: {ids_to_delete}")

    start = time.time()
    result = call_tool("delete_many", {
        "collection": "emails",
        "filter": {"_id": {"$in": ids_to_delete}}
    })
    elapsed = time.time() - start

    deleted = result.get("deleted_count", 0) if isinstance(result, dict) else 0
    print(f"         delete_many took {elapsed:.2f}s")
    print(f"         Deleted {deleted} documents")

    print("\n" + "=" * 60)
    print("SUCCESS!")
    print("=" * 60)


if __name__ == "__main__":
    main()
