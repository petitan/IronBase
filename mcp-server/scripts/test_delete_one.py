#!/usr/bin/env python3
"""
Test delete_one with a SINGLE _id - simplest case.
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
        "clientInfo": {"name": "delete-one-test", "version": "1.0"}
    })

    print("=" * 60)
    print("Testing delete_one with single _id")
    print("=" * 60)

    # Step 1: Find ONE duplicate and get the _id to delete
    print("\n[Step 1] Finding one duplicate to delete...")
    pipeline = [
        {"$group": {"_id": "$message_id", "count": {"$sum": 1}}},
        {"$match": {"count": {"$gt": 1}}},
        {"$limit": 1}
    ]
    result = call_tool("aggregate", {"collection": "emails", "pipeline": pipeline})
    dup = result.get("results", [])[0]
    message_id = dup["_id"]
    print(f"         Duplicate message_id: {message_id[:50]}...")

    # Get _ids
    result = call_tool("find", {
        "collection": "emails",
        "filter": {"message_id": message_id},
        "projection": {"_id": 1}
    })
    ids = [doc["_id"] for doc in result.get("documents", [])]
    id_to_delete = sorted(ids)[1]  # Delete the second one (keep first)
    print(f"         Will delete _id: {id_to_delete}")

    # Step 2: Test delete_one with exact _id
    print("\n[Step 2] Calling delete_one({\"_id\": %s})..." % id_to_delete)

    start = time.time()
    result = call_tool("delete_one", {
        "collection": "emails",
        "filter": {"_id": id_to_delete}
    })
    elapsed = time.time() - start

    deleted = result.get("deleted_count", 0) if isinstance(result, dict) else 0
    print(f"         delete_one took {elapsed:.2f}s")
    print(f"         Deleted {deleted} documents")

    print("\n" + "=" * 60)
    print("SUCCESS!" if deleted > 0 else "FAILED - nothing deleted")
    print("=" * 60)


if __name__ == "__main__":
    main()
