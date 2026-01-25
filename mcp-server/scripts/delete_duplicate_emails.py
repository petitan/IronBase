#!/usr/bin/env python3
"""
Delete duplicate emails - FAST version using aggregation + delete_many with $in

This version:
1. Uses $group aggregation to find duplicate message_ids (index-based, ~5 sec)
2. For each duplicate message_id, fetches the _ids and keeps the first
3. Uses delete_many with $in for fast O(k) deletion

Expected: ~3,032 duplicates to delete
"""

import json
import requests
import urllib3

urllib3.disable_warnings(urllib3.exceptions.InsecureRequestWarning)

MCP_URL = "https://localhost:8080/mcp"
HEADERS = {"Content-Type": "application/json"}
request_id = 0

def call_mcp(method: str, params: dict = None):
    global request_id
    request_id += 1
    payload = {"jsonrpc": "2.0", "id": request_id, "method": method, "params": params or {}}
    resp = requests.post(MCP_URL, json=payload, headers=HEADERS, verify=False, timeout=600)
    return resp.json()

def call_tool(name: str, arguments: dict):
    result = call_mcp("tools/call", {"name": name, "arguments": arguments})
    if "result" in result and "content" in result["result"]:
        text = result["result"]["content"][0].get("text", "")
        try:
            return json.loads(text)
        except:
            return text
    return result

def main():
    call_mcp("initialize", {"protocolVersion": "2024-11-05", "capabilities": {}, "clientInfo": {"name": "dedup", "version": "1.0"}})

    print("=" * 60)
    print("DUPLICATE EMAIL DELETION (Aggregation + Fast Delete)")
    print("=" * 60)

    # Step 1: Count before
    print("\n1. Counting emails before...")
    result = call_tool("document_count", {"collection": "emails", "filter": {}})
    count_before = result.get("count", 0) if isinstance(result, dict) else 0
    print(f"   Total: {count_before}")

    # Step 2: Find duplicate message_ids using aggregation (index-based, FAST)
    print("\n2. Finding duplicates via aggregation...")
    pipeline = [
        {"$group": {"_id": "$message_id", "count": {"$sum": 1}}},
        {"$match": {"count": {"$gt": 1}}}
    ]
    result = call_tool("aggregate", {"collection": "emails", "pipeline": pipeline})
    duplicates = result.get("results", []) if isinstance(result, dict) else []
    print(f"   Found {len(duplicates)} message_ids with duplicates")

    if not duplicates:
        print("\n   No duplicates found!")
        return

    # Calculate expected deletions
    total_to_delete = sum(d["count"] - 1 for d in duplicates)
    print(f"   Expected deletions: {total_to_delete}")

    # Step 3: For each duplicate message_id, get _ids and collect those to delete
    print("\n3. Collecting _ids to delete...")
    all_ids_to_delete = []

    for i, dup in enumerate(duplicates):
        msg_id = dup["_id"]

        # Find all docs with this message_id
        result = call_tool("find", {
            "collection": "emails",
            "filter": {"message_id": msg_id},
            "projection": {"_id": 1},
            "limit": 1000  # Should be enough for duplicates
        })
        docs = result.get("documents", []) if isinstance(result, dict) else []

        if len(docs) > 1:
            # Sort by _id and keep first, delete rest
            ids = sorted([d["_id"] for d in docs if "_id" in d])
            all_ids_to_delete.extend(ids[1:])

        if (i + 1) % 500 == 0:
            print(f"   Progress: {i+1}/{len(duplicates)}, collected {len(all_ids_to_delete)} ids")

    print(f"   Total _ids to delete: {len(all_ids_to_delete)}")

    if not all_ids_to_delete:
        print("\n   Nothing to delete!")
        return

    # Step 4: Delete in batches (fast path uses O(k) lookup per batch)
    print("\n4. Deleting duplicates...")
    BATCH_SIZE = 500
    total_deleted = 0

    for i in range(0, len(all_ids_to_delete), BATCH_SIZE):
        batch = all_ids_to_delete[i:i + BATCH_SIZE]
        result = call_tool("delete_many", {
            "collection": "emails",
            "filter": {"_id": {"$in": batch}}
        })
        deleted = result.get("deleted_count", 0) if isinstance(result, dict) else 0
        total_deleted += deleted
        print(f"   Batch {i//BATCH_SIZE + 1}: deleted {deleted} (total: {total_deleted})")

    # Step 5: Verify
    print("\n5. Verifying...")
    result = call_tool("document_count", {"collection": "emails", "filter": {}})
    count_after = result.get("count", 0) if isinstance(result, dict) else 0
    print(f"   Total after: {count_after}")
    print(f"   Difference: {count_before - count_after}")

    print("\n" + "=" * 60)
    if count_before - count_after == total_to_delete:
        print("SUCCESS! All duplicates deleted.")
    else:
        print(f"WARNING: Expected to delete {total_to_delete}, actual: {count_before - count_after}")
    print("=" * 60)

if __name__ == "__main__":
    main()
