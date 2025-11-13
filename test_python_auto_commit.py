#!/usr/bin/env python3
"""Test Python API for auto-commit modes"""

import os
from ironbase import IronBase

def test_safe_mode_default():
    """Test that Safe mode is the default"""
    db_path = "test_py_safe.mlite"
    wal_path = "test_py_safe.wal"

    # Cleanup
    for f in [db_path, wal_path]:
        if os.path.exists(f):
            os.remove(f)

    print("=== Test 1: Safe Mode (Default) ===")

    # Open database (default = safe mode)
    db = IronBase(db_path)
    col = db.collection("users")

    # Insert document
    result = col.insert_one({"name": "Alice", "age": 30})
    print(f"  Inserted ID: {result['inserted_id']}")

    # Count documents
    count = col.count_documents({})
    print(f"  Document count: {count}")
    assert count == 1, f"Expected 1 document, got {count}"

    db.close()
    print("  ✓ Safe mode test passed\n")

    # Cleanup
    for f in [db_path, wal_path]:
        if os.path.exists(f):
            os.remove(f)


def test_batch_mode():
    """Test Batch mode"""
    db_path = "test_py_batch.mlite"
    wal_path = "test_py_batch.wal"

    # Cleanup
    for f in [db_path, wal_path]:
        if os.path.exists(f):
            os.remove(f)

    print("=== Test 2: Batch Mode ===")

    # Open database in batch mode
    db = IronBase(db_path, durability="batch", batch_size=5)
    col = db.collection("test")

    # Insert 10 documents (should trigger 2 flushes)
    for i in range(10):
        col.insert_one({"value": i})

    count = col.count_documents({})
    print(f"  Inserted {count} documents")
    assert count == 10, f"Expected 10 documents, got {count}"

    db.close()
    print("  ✓ Batch mode test passed\n")

    # Cleanup
    for f in [db_path, wal_path]:
        if os.path.exists(f):
            os.remove(f)


def test_unsafe_mode():
    """Test Unsafe mode"""
    db_path = "test_py_unsafe.mlite"
    wal_path = "test_py_unsafe.wal"

    # Cleanup
    for f in [db_path, wal_path]:
        if os.path.exists(f):
            os.remove(f)

    print("=== Test 3: Unsafe Mode ===")

    # Open database in unsafe mode
    db = IronBase(db_path, durability="unsafe")
    col = db.collection("test")

    # Insert documents (fast path, no WAL)
    for i in range(100):
        col.insert_one({"value": i})

    count = col.count_documents({})
    print(f"  Inserted {count} documents (fast path)")
    assert count == 100, f"Expected 100 documents, got {count}"

    # Manual checkpoint
    db.checkpoint()

    db.close()
    print("  ✓ Unsafe mode test passed\n")

    # Cleanup
    for f in [db_path, wal_path]:
        if os.path.exists(f):
            os.remove(f)


def test_invalid_durability():
    """Test invalid durability mode"""
    db_path = "test_py_invalid.mlite"

    print("=== Test 4: Invalid Durability Mode ===")

    try:
        db = IronBase(db_path, durability="invalid")
        print("  ✗ Should have raised ValueError")
        assert False
    except ValueError as e:
        print(f"  ✓ Correctly raised ValueError: {e}\n")

    # Cleanup
    if os.path.exists(db_path):
        os.remove(f)


if __name__ == "__main__":
    print("=" * 60)
    print("  PYTHON AUTO-COMMIT MODE TESTS")
    print("=" * 60)
    print()

    test_safe_mode_default()
    test_batch_mode()
    test_unsafe_mode()
    test_invalid_durability()

    print("=" * 60)
    print("🎉 ALL PYTHON AUTO-COMMIT TESTS PASSED!")
    print("=" * 60)
