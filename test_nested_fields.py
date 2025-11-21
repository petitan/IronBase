#!/usr/bin/env python3
"""Python integration tests for nested field (dot-notation) support."""

import os
from ironbase import IronBase


def cleanup(path: str):
    for ext in (".mlite", ".wal"):
        try:
            os.remove(path.replace(".mlite", ext))
        except FileNotFoundError:
            pass


def test_nested_query_operations():
    print("=== Nested Query Operations ===")
    db_path = "test_nested_query.mlite"
    cleanup(db_path)

    db = IronBase(db_path)
    users = db.collection("users")

    users.insert_many([
        {
            "name": "Anna",
            "address": {"city": "Budapest", "zip": 1111},
            "stats": {"login_count": 42, "flags": {"beta": True}},
        },
        {
            "name": "Bence",
            "address": {"city": "Debrecen", "zip": 4025},
            "stats": {"login_count": 5, "flags": {"beta": False}},
        },
    ])

    anna = users.find_one({"address.city": "Budapest"})
    assert anna and anna["name"] == "Anna"

    power_users = users.find({"stats.login_count": {"$gte": 10}})
    assert len(power_users) == 1 and power_users[0]["name"] == "Anna"

    db.close()
    cleanup(db_path)
    print("✓ Nested query operations passed\n")


if __name__ == "__main__":
    print("=" * 70)
    print(" NESTED FIELD INTEGRATION TESTS ")
    print("=" * 70)

    test_nested_query_operations()

    print("=" * 70)
    print("🎉 ALL NESTED TESTS PASSED")
    print("=" * 70)
