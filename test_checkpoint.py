#!/usr/bin/env python3
"""Test WAL checkpoint functionality"""

import os
from ironbase import IronBase

def test_checkpoint():
    """Test that checkpoint prevents WAL growth"""
    db_path = "test_checkpoint.mlite"
    wal_path = "test_checkpoint.wal"

    # Clean up
    for f in [db_path, wal_path]:
        if os.path.exists(f):
            os.remove(f)

    print("=== WAL Checkpoint Test ===\n")

    # Create database and get collection
    db = IronBase(db_path)
    col = db.collection("test")

    print("Phase 1: Insert 1000 documents WITHOUT checkpoint")
    for i in range(1000):
        col.insert_one({"value": i})

    # Check WAL size before checkpoint
    wal_size_before = os.path.getsize(wal_path) if os.path.exists(wal_path) else 0
    print(f"  WAL size before checkpoint: {wal_size_before} bytes")

    # Checkpoint should clear WAL
    print("\nPhase 2: Call checkpoint()")
    db.checkpoint()

    # Check WAL size after checkpoint
    wal_size_after = os.path.getsize(wal_path) if os.path.exists(wal_path) else 0
    print(f"  WAL size after checkpoint: {wal_size_after} bytes")

    if wal_size_after == 0:
        print("  ✓ WAL cleared successfully!")
    else:
        print(f"  ✗ WAL not cleared (still {wal_size_after} bytes)")

    # Insert more documents
    print("\nPhase 3: Insert 1000 more documents")
    for i in range(1000, 2000):
        col.insert_one({"value": i})

    wal_size_after_insert = os.path.getsize(wal_path) if os.path.exists(wal_path) else 0
    print(f"  WAL size after 1000 more inserts: {wal_size_after_insert} bytes")

    # Checkpoint again
    print("\nPhase 4: Call checkpoint() again")
    db.checkpoint()

    wal_size_final = os.path.getsize(wal_path) if os.path.exists(wal_path) else 0
    print(f"  WAL size after second checkpoint: {wal_size_final} bytes")

    # Verify data integrity
    print("\nPhase 5: Verify data integrity")
    count = col.count_documents({})
    print(f"  Total documents: {count}")

    if count == 2000:
        print("  ✓ All documents preserved!")
    else:
        print(f"  ✗ Expected 2000 documents, got {count}")

    db.close()

    # Clean up
    for f in [db_path, wal_path]:
        if os.path.exists(f):
            os.remove(f)

    print("\n=== Test Complete ===")

if __name__ == "__main__":
    test_checkpoint()
