#!/usr/bin/env python3
"""
Remove all `level` fields using only IronBase CRUD operations.

Example:
    python examples/remove_level_via_api.py \
        --db mini_schema.mlite \
        --collection documents \
        --document /home/petitan/teszt2/ISO17025_MK_NESTED.json \
        --schema /home/petitan/docjl/schema/docjl-schema.json \
        --output cleaned.json \
        --cleanup
"""

import argparse
import json
import os
from typing import Dict, List, Set

from ironbase import IronBase


def cleanup(path: str) -> None:
    for ext in (".mlite", ".wal"):
        target = path.replace(".mlite", ext)
        if os.path.exists(target):
            os.remove(target)


def collect_level_paths(node, prefix: str, paths: Set[str]) -> None:
    if isinstance(node, dict):
        if "level" in node:
            key = f"{prefix}level" if prefix else "level"
            paths.add(key)
        for key, value in node.items():
            new_prefix = f"{prefix}{key}." if prefix else f"{key}."
            collect_level_paths(value, new_prefix, paths)
    elif isinstance(node, list):
        for idx, item in enumerate(node):
            new_prefix = f"{prefix}{idx}." if prefix else f"{idx}."
            collect_level_paths(item, new_prefix, paths)


def main() -> None:
    parser = argparse.ArgumentParser(description="Remove all `level` fields via IronBase updates.")
    parser.add_argument("--db", required=True, help="Database path (.mlite)")
    parser.add_argument("--collection", default="documents", help="Collection name")
    parser.add_argument("--document", required=True, help="Path to JSON document to insert")
    parser.add_argument("--schema", help="Optional JSON schema path")
    parser.add_argument("--output", help="Optional JSON file to write cleaned document")
    parser.add_argument("--cleanup", action="store_true", help="Delete existing DB/WAL before running")
    args = parser.parse_args()

    if args.cleanup:
        cleanup(args.db)

    db = IronBase(args.db)
    collection = db.collection(args.collection)

    if args.schema:
        with open(args.schema, "r", encoding="utf-8") as schema_file:
            schema = json.load(schema_file)
        collection.set_schema(schema)

    with open(args.document, "r", encoding="utf-8") as doc_file:
        document = json.load(doc_file)

    insert_result = collection.insert_one(document)
    inserted_id = insert_result["inserted_id"]

    stored = collection.find_one({"_id": inserted_id})
    if stored is None:
        raise RuntimeError("Document not found after insert")

    level_paths: Set[str] = set()
    collect_level_paths(stored, "", level_paths)

    if not level_paths:
        print("No `level` fields found.")
    else:
        unset_doc: Dict[str, str] = {path: "" for path in sorted(level_paths)}
        collection.update_one({"_id": inserted_id}, {"$unset": unset_doc})
        print(f"Removed {len(level_paths)} `level` fields via update.")

    if args.output:
        cleaned = collection.find_one({"_id": inserted_id})
        if cleaned:
            cleaned.pop("_id", None)
            cleaned.pop("_collection", None)
            with open(args.output, "w", encoding="utf-8") as out_file:
                json.dump(cleaned, out_file, ensure_ascii=False, indent=2)
            print(f"Cleaned document written to {args.output}")

    db.close()


if __name__ == "__main__":
    main()
