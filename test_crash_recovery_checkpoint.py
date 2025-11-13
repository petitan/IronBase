#!/usr/bin/env python3
"""Test crash recovery with WAL checkpoint"""

import os
import sys
from ironbase import IronBase

def test_crash_before_checkpoint():
    """Test crash BEFORE checkpoint - WAL should recover data"""
    db_path = "test_crash1.mlite"
    wal_path = "test_crash1.wal"

    # Cleanup
    for f in [db_path, wal_path]:
        if os.path.exists(f):
            os.remove(f)

    print("=== Scenario 1: Crash BEFORE Checkpoint ===\n")

    # Phase 1: Write data but DON'T checkpoint
    print("Phase 1: Insert 100 documents WITHOUT checkpoint")
    db = IronBase(db_path)
    col = db.collection("test")

    for i in range(100):
        col.insert_one({"value": i, "data": f"Entry {i}"})

    wal_size = os.path.getsize(wal_path) if os.path.exists(wal_path) else 0
    print(f"  WAL size: {wal_size} bytes")
    print(f"  Document count: {col.count_documents({})}")

    # Simulate crash - DON'T call close()
    print("\n  💥 SIMULATED CRASH (no close, no checkpoint)")
    del col
    del db

    # Phase 2: Reopen and check recovery
    print("\nPhase 2: Reopen database (WAL recovery should happen)")
    db2 = IronBase(db_path)
    col2 = db2.collection("test")

    count_after = col2.count_documents({})
    print(f"  Document count after recovery: {count_after}")

    if count_after == 100:
        print("  ✓ All 100 documents recovered from WAL!")
    else:
        print(f"  ✗ FAILED: Expected 100, got {count_after}")
        return False

    # Check WAL cleared after recovery
    wal_size_after = os.path.getsize(wal_path) if os.path.exists(wal_path) else 0
    print(f"  WAL size after recovery: {wal_size_after} bytes")

    if wal_size_after == 0:
        print("  ✓ WAL cleared after recovery!")
    else:
        print(f"  ⚠ WAL not cleared ({wal_size_after} bytes)")

    db2.close()

    # Cleanup
    for f in [db_path, wal_path]:
        if os.path.exists(f):
            os.remove(f)

    print("  ✓ Scenario 1 PASSED\n")
    return True


def test_crash_after_checkpoint():
    """Test crash AFTER checkpoint - data in DB, WAL empty"""
    db_path = "test_crash2.mlite"
    wal_path = "test_crash2.wal"

    # Cleanup
    for f in [db_path, wal_path]:
        if os.path.exists(f):
            os.remove(f)

    print("=== Scenario 2: Crash AFTER Checkpoint ===\n")

    # Phase 1: Write data and checkpoint
    print("Phase 1: Insert 100 documents WITH checkpoint")
    db = IronBase(db_path)
    col = db.collection("test")

    for i in range(100):
        col.insert_one({"value": i, "data": f"Entry {i}"})

    print(f"  Document count before checkpoint: {col.count_documents({})}")

    # Checkpoint - this should flush to DB and clear WAL
    db.checkpoint()

    wal_size = os.path.getsize(wal_path) if os.path.exists(wal_path) else 0
    print(f"  WAL size after checkpoint: {wal_size} bytes")

    if wal_size == 0:
        print("  ✓ WAL cleared by checkpoint")
    else:
        print(f"  ✗ WAL not cleared ({wal_size} bytes)")

    # Simulate crash AFTER checkpoint
    print("\n  💥 SIMULATED CRASH (after checkpoint)")
    del col
    del db

    # Phase 2: Reopen and verify data
    print("\nPhase 2: Reopen database (should load from DB, not WAL)")
    db2 = IronBase(db_path)
    col2 = db2.collection("test")

    count_after = col2.count_documents({})
    print(f"  Document count after reopen: {count_after}")

    if count_after == 100:
        print("  ✓ All 100 documents loaded from DB!")
    else:
        print(f"  ✗ FAILED: Expected 100, got {count_after}")
        return False

    db2.close()

    # Cleanup
    for f in [db_path, wal_path]:
        if os.path.exists(f):
            os.remove(f)

    print("  ✓ Scenario 2 PASSED\n")
    return True


def test_crash_between_writes():
    """Test crash in middle of write cycle"""
    db_path = "test_crash3.mlite"
    wal_path = "test_crash3.wal"

    # Cleanup
    for f in [db_path, wal_path]:
        if os.path.exists(f):
            os.remove(f)

    print("=== Scenario 3: Crash Between Write Cycles ===\n")

    # Phase 1: Write batch 1, checkpoint, write batch 2, crash
    print("Phase 1: Write 50 docs → checkpoint → write 50 more → crash")
    db = IronBase(db_path)
    col = db.collection("test")

    # Batch 1
    for i in range(50):
        col.insert_one({"batch": 1, "value": i})

    print(f"  After batch 1: {col.count_documents({})} documents")

    # Checkpoint
    db.checkpoint()
    print("  ✓ Checkpoint done")

    # Batch 2 (in WAL, not checkpointed)
    for i in range(50, 100):
        col.insert_one({"batch": 2, "value": i})

    print(f"  After batch 2: {col.count_documents({})} documents")

    wal_size = os.path.getsize(wal_path) if os.path.exists(wal_path) else 0
    print(f"  WAL size before crash: {wal_size} bytes")

    # Crash without checkpoint
    print("\n  💥 SIMULATED CRASH (batch 2 only in WAL)")
    del col
    del db

    # Phase 2: Recovery
    print("\nPhase 2: Reopen (should recover batch 1 from DB + batch 2 from WAL)")
    db2 = IronBase(db_path)
    col2 = db2.collection("test")

    count_total = col2.count_documents({})
    count_batch1 = col2.count_documents({"batch": 1})
    count_batch2 = col2.count_documents({"batch": 2})

    print(f"  Total documents: {count_total}")
    print(f"  Batch 1 (from DB): {count_batch1}")
    print(f"  Batch 2 (from WAL): {count_batch2}")

    if count_total == 100 and count_batch1 == 50 and count_batch2 == 50:
        print("  ✓ All 100 documents recovered (50 from DB + 50 from WAL)!")
    else:
        print(f"  ✗ FAILED: Expected 100 (50+50), got {count_total} ({count_batch1}+{count_batch2})")
        return False

    db2.close()

    # Cleanup
    for f in [db_path, wal_path]:
        if os.path.exists(f):
            os.remove(f)

    print("  ✓ Scenario 3 PASSED\n")
    return True


if __name__ == "__main__":
    print("=" * 60)
    print("  WAL CRASH RECOVERY TEST WITH CHECKPOINT")
    print("=" * 60)
    print()

    results = []

    # Run all scenarios
    results.append(("Crash BEFORE checkpoint", test_crash_before_checkpoint()))
    results.append(("Crash AFTER checkpoint", test_crash_after_checkpoint()))
    results.append(("Crash BETWEEN write cycles", test_crash_between_writes()))

    # Summary
    print("=" * 60)
    print("  SUMMARY")
    print("=" * 60)

    for name, passed in results:
        status = "✓ PASS" if passed else "✗ FAIL"
        print(f"  {status}: {name}")

    all_passed = all(r[1] for r in results)

    print()
    if all_passed:
        print("🎉 ALL CRASH RECOVERY TESTS PASSED!")
        sys.exit(0)
    else:
        print("❌ SOME TESTS FAILED")
        sys.exit(1)
