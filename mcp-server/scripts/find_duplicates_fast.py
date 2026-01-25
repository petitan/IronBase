#!/usr/bin/env python3
"""
Delete duplicate emails - FAST version using aggregation pipeline

Strategy (now that index-based aggregation works):
1. Use $group + $match to find duplicate message_ids (index-based, ~5s)
2. For each duplicate, fetch _ids using indexed query
3. Delete all but the first _id

Performance: ~30 seconds for 130K emails with 3K duplicates
"""

import json
import requests
import urllib3
import time
from typing import Any

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


def call_tool(name: str, arguments: dict) -> Any:
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


def find_duplicates_with_aggregation() -> list[dict]:
    """
    Use aggregation pipeline to find duplicate message_ids.
    Uses index-based $group optimization (fixed in 2026-01).
    """
    pipeline = [
        {"$group": {"_id": "$message_id", "count": {"$sum": 1}}},
        {"$match": {"count": {"$gt": 1}}}
    ]

    result = call_tool("aggregate", {
        "collection": "emails",
        "pipeline": pipeline
    })

    if isinstance(result, dict) and "results" in result:
        return result["results"]
    return []


def get_ids_for_message_id(message_id: str) -> list[str]:
    """Get all _ids for a given message_id (uses index)."""
    result = call_tool("find", {
        "collection": "emails",
        "filter": {"message_id": message_id},
        "projection": {"_id": 1}
    })

    if isinstance(result, dict) and "documents" in result:
        return [doc["_id"] for doc in result["documents"] if isinstance(doc, dict)]
    return []


def main():
    start_time = time.time()

    # Initialize MCP session
    call_mcp("initialize", {
        "protocolVersion": "2024-11-05",
        "capabilities": {},
        "clientInfo": {"name": "dedup-fast", "version": "4.0"}
    })

    print("=" * 60)
    print("Email Duplicate Finder - Aggregation Pipeline Version")
    print("=" * 60)

    # Step 1: Find duplicate message_ids using aggregation (index-based)
    print("\n[Step 1] Finding duplicates via aggregation (index-based)...")
    agg_start = time.time()

    duplicates = find_duplicates_with_aggregation()

    agg_time = time.time() - agg_start
    print(f"         Found {len(duplicates)} duplicate message_ids in {agg_time:.2f}s")

    if not duplicates:
        print("\nNo duplicates found!")
        return

    # Show top 10 duplicates
    duplicates_sorted = sorted(duplicates, key=lambda x: x.get("count", 0), reverse=True)
    print(f"\n         Top duplicates:")
    for dup in duplicates_sorted[:10]:
        msg_id = str(dup['_id'])[:50] if dup['_id'] else 'null'
        print(f"           {dup['count']:3d}x  {msg_id}...")

    # Step 2: For each duplicate, get _ids and collect those to delete
    print(f"\n[Step 2] Collecting _ids to delete...")
    fetch_start = time.time()

    all_ids_to_delete = []
    total_duplicates = len(duplicates)

    for i, dup in enumerate(duplicates, 1):
        message_id = dup["_id"]
        if not message_id:
            continue

        ids = get_ids_for_message_id(message_id)
        if len(ids) > 1:
            # Keep the first (smallest _id), delete the rest
            ids_sorted = sorted(ids)
            all_ids_to_delete.extend(ids_sorted[1:])

        if i % 500 == 0 or i == total_duplicates:
            print(f"         Processed {i}/{total_duplicates} duplicate groups...")

    fetch_time = time.time() - fetch_start
    print(f"         Collected {len(all_ids_to_delete)} documents to delete in {fetch_time:.2f}s")

    if not all_ids_to_delete:
        print("\nNothing to delete!")
        return

    # Step 3: Delete in batches
    print(f"\n[Step 3] Deleting {len(all_ids_to_delete)} duplicate documents...")
    delete_start = time.time()

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
        batch_num = i // BATCH_SIZE + 1
        total_batches = (len(all_ids_to_delete) + BATCH_SIZE - 1) // BATCH_SIZE
        print(f"         Batch {batch_num}/{total_batches}: deleted {deleted}")

    delete_time = time.time() - delete_start
    total_time = time.time() - start_time

    print("\n" + "=" * 60)
    print("Summary")
    print("=" * 60)
    print(f"  Duplicate groups found:  {len(duplicates)}")
    print(f"  Documents deleted:       {total_deleted}")
    print(f"  Aggregation time:        {agg_time:.2f}s")
    print(f"  Fetch IDs time:          {fetch_time:.2f}s")
    print(f"  Delete time:             {delete_time:.2f}s")
    print(f"  Total time:              {total_time:.2f}s")
    print("=" * 60)


if __name__ == "__main__":
    main()
