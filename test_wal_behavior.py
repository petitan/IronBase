#!/usr/bin/env python3
"""Detailed WAL behavior analysis"""

import os
import time
from ironbase import IronBase

def check_wal_size(wal_path):
    """Get WAL file size"""
    if os.path.exists(wal_path):
        size = os.path.getsize(wal_path)
        return size
    return 0

def test_wal_timing():
    """Test EXACTLY when WAL is written and cleared"""
    db_path = "test_wal_timing.mlite"
    wal_path = "test_wal_timing.wal"

    # Cleanup
    for f in [db_path, wal_path]:
        if os.path.exists(f):
            os.remove(f)

    print("=== DETAILED WAL TIMING TEST ===\n")

    # Open database
    print("Step 1: Open database")
    db = IronBase(db_path)
    col = db.collection("test")
    print(f"  WAL size after open: {check_wal_size(wal_path)} bytes\n")

    # Insert ONE document
    print("Step 2: Insert ONE document")
    col.insert_one({"value": 1})
    wal_after_insert = check_wal_size(wal_path)
    print(f"  WAL size IMMEDIATELY after insert: {wal_after_insert} bytes")

    if wal_after_insert > 0:
        print("  ✓ WAL contains data!")
    else:
        print("  ⚠ WAL is EMPTY (flush already called?)")

    # Wait a bit
    time.sleep(0.1)
    print(f"  WAL size after 100ms: {check_wal_size(wal_path)} bytes\n")

    # Insert another document WITHOUT any explicit flush
    print("Step 3: Insert SECOND document (no explicit flush)")
    col.insert_one({"value": 2})
    wal_after_insert2 = check_wal_size(wal_path)
    print(f"  WAL size after 2nd insert: {wal_after_insert2} bytes")

    if wal_after_insert2 > wal_after_insert:
        print("  ✓ WAL growing (appending entries)")
    elif wal_after_insert2 == 0:
        print("  ⚠ WAL cleared between inserts!")
    print()

    # Count documents (might trigger flush?)
    print("Step 4: Count documents (read operation)")
    count = col.count_documents({})
    print(f"  Document count: {count}")
    print(f"  WAL size after count: {check_wal_size(wal_path)} bytes\n")

    # Don't close, don't checkpoint - just delete collection reference
    print("Step 5: Delete collection reference (no explicit close)")
    del col
    wal_after_del_col = check_wal_size(wal_path)
    print(f"  WAL size after 'del col': {wal_after_del_col} bytes")

    if wal_after_del_col == 0:
        print("  ⚠ WAL cleared when collection dropped!")
    print()

    # Delete database reference
    print("Step 6: Delete database reference (no explicit close)")
    del db
    wal_after_del_db = check_wal_size(wal_path)
    print(f"  WAL size after 'del db': {wal_after_del_db} bytes")

    if wal_after_del_db == 0:
        print("  ⚠ WAL cleared when database dropped (Drop trait called flush!)")
    print()

    # Reopen without cleanup
    print("Step 7: Reopen database (check if WAL had data)")
    db2 = IronBase(db_path)
    col2 = db2.collection("test")
    count2 = col2.count_documents({})
    print(f"  Document count after reopen: {count2}")

    if count2 == 2:
        print("  ✓ Both documents persisted")
    else:
        print(f"  ✗ Expected 2, got {count2}")

    print(f"  WAL size after reopen: {check_wal_size(wal_path)} bytes")

    db2.close()

    # Cleanup
    for f in [db_path, wal_path]:
        if os.path.exists(f):
            os.remove(f)

    print("\n=== Test Complete ===")


def test_without_del():
    """Test keeping references alive"""
    db_path = "test_wal_nodelete.mlite"
    wal_path = "test_wal_nodelete.wal"

    # Cleanup
    for f in [db_path, wal_path]:
        if os.path.exists(f):
            os.remove(f)

    print("\n\n=== TEST WITHOUT DELETING REFERENCES ===\n")

    print("Step 1: Open and insert")
    db = IronBase(db_path)
    col = db.collection("test")

    col.insert_one({"value": 1})
    print(f"  WAL after insert 1: {check_wal_size(wal_path)} bytes")

    col.insert_one({"value": 2})
    print(f"  WAL after insert 2: {check_wal_size(wal_path)} bytes")

    col.insert_one({"value": 3})
    print(f"  WAL after insert 3: {check_wal_size(wal_path)} bytes")

    print("\nStep 2: Explicit close()")
    db.close()
    print(f"  WAL after close(): {check_wal_size(wal_path)} bytes")

    # Cleanup
    for f in [db_path, wal_path]:
        if os.path.exists(f):
            os.remove(f)


if __name__ == "__main__":
    test_wal_timing()
    test_without_del()
