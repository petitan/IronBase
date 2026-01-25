#!/usr/bin/env python3
"""
Test delete_many with SMALL batch (5 items) to isolate the issue.
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
    resp = requests.post(MCP_URL, json=payload, headers=HEADERS, verify=False, timeout=300)
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
        "clientInfo": {"name": "delete-many-test", "version": "1.0"}
    })

    print("=" * 60)
    print("Testing delete_many with small batch")
    print("=" * 60)

    # Step 1: Find 3 duplicates
    print("\n[Step 1] Finding 3 duplicates...")
    pipeline = [
        {"$group": {"_id": "$message_id", "count": {"$sum": 1}}},
        {"$match": {"count": {"$gt": 1}}},
        {"$limit": 3}
    ]
    result = call_tool("aggregate", {"collection": "emails", "pipeline": pipeline})
    duplicates = result.get("results", [])
    print(f"         Found {len(duplicates)} duplicate groups")

    # Step 2: Collect _ids to delete
    all_ids_to_delete = []
    for dup in duplicates:
        message_id = dup["_id"]
        result = call_tool("find", {
            "collection": "emails",
            "filter": {"message_id": message_id},
            "projection": {"_id": 1}
        })
        ids = [doc["_id"] for doc in result.get("documents", [])]
        if len(ids) > 1:
            all_ids_to_delete.extend(sorted(ids)[1:])

    print(f"         Will delete {len(all_ids_to_delete)} documents")
    print(f"         IDs: {all_ids_to_delete}")

    if not all_ids_to_delete:
        print("Nothing to delete!")
        return

    # Step 3: Delete with small batch using $in
    print(f"\n[Step 3] Calling delete_many with $in [{len(all_ids_to_delete)} ids]...")

    start = time.time()
    result = call_tool("delete_many", {
        "collection": "emails",
        "filter": {"_id": {"$in": all_ids_to_delete}}
    })
    elapsed = time.time() - start

    deleted = result.get("deleted_count", 0) if isinstance(result, dict) else 0
    print(f"         delete_many took {elapsed:.2f}s")
    print(f"         Deleted {deleted} documents")

    print("\n" + "=" * 60)
    print("SUCCESS!" if deleted == len(all_ids_to_delete) else f"PARTIAL: expected {len(all_ids_to_delete)}, got {deleted}")
    print("=" * 60)


if __name__ == "__main__":
    main()
