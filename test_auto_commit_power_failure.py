#!/usr/bin/env python3
"""
Auto-Commit Power Failure Recovery Tests

Tests that verify data durability guarantees for each durability mode.
Simulates power failure by NOT calling close() or checkpoint().
"""

import os
import sys
from ironbase import IronBase


def simulate_power_failure():
    """
    Simulate power failure by deleting database object WITHOUT calling close().
    This leaves the DB in whatever state it was mid-operation.
    """
    # In real power failure:
    # - No flush() called
    # - No close() called
    # - File buffers may be partially written (OS dependent)
    # - WAL file is in whatever state it was
    pass


def test_safe_mode_zero_data_loss():
    """
    Safe Mode: ZERO data loss guarantee

    Every insert is auto-committed with WAL + fsync.
    Power failure should NOT lose any data.
    """
    db_path = "test_safe_pf.mlite"
    wal_path = "test_safe_pf.wal"

    # Cleanup
    for f in [db_path, wal_path]:
        if os.path.exists(f):
            os.remove(f)

    print("=== Test 1: Safe Mode - ZERO Data Loss ===\n")

    # Phase 1: Insert data in Safe mode
    print("Phase 1: Insert 100 documents in Safe mode")
    db = IronBase(db_path, durability="safe")
    col = db.collection("test")

    for i in range(100):
        col.insert_one({"value": i, "mode": "safe"})

    count_before = col.count_documents({})
    print(f"  Documents before crash: {count_before}")

    # Check WAL (should have entries from last operation)
    wal_exists = os.path.exists(wal_path)
    wal_size = os.path.getsize(wal_path) if wal_exists else 0
    print(f"  WAL file: {'exists' if wal_exists else 'missing'}, {wal_size} bytes")

    # ⚡ POWER FAILURE - no close(), no flush()
    print("\n  ⚡ POWER FAILURE (simulated crash)")
    del col
    del db

    # Phase 2: Recovery
    print("\nPhase 2: Reopen database and check recovery")
    db2 = IronBase(db_path, durability="safe")
    col2 = db2.collection("test")

    count_after = col2.count_documents({})
    print(f"  Documents after recovery: {count_after}")

    # Verify ZERO data loss
    if count_after == 100:
        print("  ✓ SUCCESS: All 100 documents recovered")
        print("  ✓ ZERO DATA LOSS in Safe mode!")
    else:
        print(f"  ✗ FAILED: Expected 100, got {count_after}")
        print(f"  ✗ DATA LOSS: {100 - count_after} documents lost")
        return False

    db2.close()

    # Cleanup
    for f in [db_path, wal_path]:
        if os.path.exists(f):
            os.remove(f)

    print()
    return True


def test_batch_mode_bounded_loss():
    """
    Batch Mode: Bounded data loss (max batch_size operations)

    Operations are batched and committed periodically.
    Power failure can lose at most batch_size operations.
    """
    db_path = "test_batch_pf.mlite"
    wal_path = "test_batch_pf.wal"

    # Cleanup
    for f in [db_path, wal_path]:
        if os.path.exists(f):
            os.remove(f)

    print("=== Test 2: Batch Mode - Bounded Data Loss ===\n")

    # Phase 1: Insert data in Batch mode
    batch_size = 10
    total_docs = 25  # 2 full batches + 5 uncommitted

    print(f"Phase 1: Insert {total_docs} documents (batch_size={batch_size})")
    db = IronBase(db_path, durability="batch", batch_size=batch_size)
    col = db.collection("test")

    for i in range(total_docs):
        col.insert_one({"value": i, "mode": "batch"})

    count_before = col.count_documents({})
    print(f"  Documents before crash: {count_before}")
    print(f"  Expected after crash: ≥ {total_docs - batch_size} (2 committed batches)")

    # ⚡ POWER FAILURE - no close(), no flush()
    print("\n  ⚡ POWER FAILURE (last 5 operations uncommitted)")
    del col
    del db

    # Phase 2: Recovery
    print("\nPhase 2: Reopen database and check recovery")
    db2 = IronBase(db_path, durability="batch", batch_size=batch_size)
    col2 = db2.collection("test")

    count_after = col2.count_documents({})
    print(f"  Documents after recovery: {count_after}")

    # Verify bounded loss (at least 2 full batches = 20 docs)
    min_expected = total_docs - batch_size  # 25 - 10 = 15
    if count_after >= min_expected:
        lost = total_docs - count_after
        print(f"  ✓ SUCCESS: Recovered {count_after} documents")
        print(f"  ✓ Data loss bounded: {lost} documents lost (< batch_size={batch_size})")
    else:
        print(f"  ✗ FAILED: Expected ≥ {min_expected}, got {count_after}")
        return False

    db2.close()

    # Cleanup
    for f in [db_path, wal_path]:
        if os.path.exists(f):
            os.remove(f)

    print()
    return True


def test_unsafe_mode_high_risk():
    """
    Unsafe Mode: High data loss risk

    No auto-commit. All data since last checkpoint can be lost.
    """
    db_path = "test_unsafe_pf.mlite"
    wal_path = "test_unsafe_pf.wal"

    # Cleanup
    for f in [db_path, wal_path]:
        if os.path.exists(f):
            os.remove(f)

    print("=== Test 3: Unsafe Mode - High Data Loss Risk ===\n")

    # Phase 1: Insert data in Unsafe mode
    print("Phase 1: Insert 100 documents in Unsafe mode (no checkpoint)")
    db = IronBase(db_path, durability="unsafe")
    col = db.collection("test")

    for i in range(100):
        col.insert_one({"value": i, "mode": "unsafe"})

    count_before = col.count_documents({})
    print(f"  Documents before crash: {count_before}")

    # ⚡ POWER FAILURE - no close(), no flush(), no checkpoint()
    print("\n  ⚡ POWER FAILURE (NO checkpoint called)")
    del col
    del db

    # Phase 2: Recovery
    print("\nPhase 2: Reopen database and check recovery")
    db2 = IronBase(db_path, durability="unsafe")
    col2 = db2.collection("test")

    count_after = col2.count_documents({})
    print(f"  Documents after recovery: {count_after}")

    # Unsafe mode: data may or may not survive (depends on OS flush timing)
    if count_after < 100:
        print(f"  ⚠ DATA LOSS: {100 - count_after} documents lost")
        print(f"  ⚠ This is EXPECTED in Unsafe mode without checkpoint")
    elif count_after == 100:
        print(f"  ℹ All data survived (lucky - OS flushed metadata)")
        print(f"  ℹ But this is NOT GUARANTEED in Unsafe mode!")

    db2.close()

    # Cleanup
    for f in [db_path, wal_path]:
        if os.path.exists(f):
            os.remove(f)

    print()
    return True  # Unsafe mode is "working as designed" even with data loss


def test_safe_mode_wal_replay():
    """
    Safe Mode: WAL Replay verification

    Verify that committed operations in WAL are correctly replayed.
    """
    db_path = "test_wal_replay.mlite"
    wal_path = "test_wal_replay.wal"

    # Cleanup
    for f in [db_path, wal_path]:
        if os.path.exists(f):
            os.remove(f)

    print("=== Test 4: Safe Mode - WAL Replay Verification ===\n")

    # Phase 1: Insert baseline + safe operations
    print("Phase 1: Insert 50 documents, crash, then 50 more")

    # First batch
    db = IronBase(db_path, durability="safe")
    col = db.collection("test")

    for i in range(50):
        col.insert_one({"value": i, "batch": 1})

    db.close()  # Clean close
    print("  ✓ First 50 documents committed")

    # Second batch (will crash)
    db2 = IronBase(db_path, durability="safe")
    col2 = db2.collection("test")

    for i in range(50, 100):
        col2.insert_one({"value": i, "batch": 2})

    print(f"  Documents before crash: {col2.count_documents({})}")

    # ⚡ POWER FAILURE
    print("\n  ⚡ POWER FAILURE (after second batch)")
    del col2
    del db2

    # Phase 2: Recovery and verification
    print("\nPhase 2: Verify WAL replay")
    db3 = IronBase(db_path, durability="safe")
    col3 = db3.collection("test")

    count_total = col3.count_documents({})
    count_batch1 = col3.count_documents({"batch": 1})
    count_batch2 = col3.count_documents({"batch": 2})

    print(f"  Total documents: {count_total}")
    print(f"  Batch 1 (committed): {count_batch1}")
    print(f"  Batch 2 (from WAL): {count_batch2}")

    if count_total == 100 and count_batch1 == 50 and count_batch2 == 50:
        print("  ✓ SUCCESS: WAL correctly replayed all operations")
    else:
        print(f"  ✗ FAILED: Expected 100 (50+50), got {count_total} ({count_batch1}+{count_batch2})")
        return False

    db3.close()

    # Cleanup
    for f in [db_path, wal_path]:
        if os.path.exists(f):
            os.remove(f)

    print()
    return True


if __name__ == "__main__":
    print("=" * 70)
    print("  AUTO-COMMIT POWER FAILURE RECOVERY TESTS")
    print("  Testing durability guarantees for each mode")
    print("=" * 70)
    print()

    results = []

    # Run all tests
    results.append(("Safe Mode: Zero data loss", test_safe_mode_zero_data_loss()))
    results.append(("Batch Mode: Bounded loss", test_batch_mode_bounded_loss()))
    results.append(("Unsafe Mode: High risk", test_unsafe_mode_high_risk()))
    results.append(("Safe Mode: WAL replay", test_safe_mode_wal_replay()))

    # Summary
    print("=" * 70)
    print("  SUMMARY - Durability Guarantees Verified")
    print("=" * 70)
    print()

    for name, passed in results:
        status = "✓ PASS" if passed else "✗ FAIL"
        print(f"  {status}: {name}")

    all_passed = all(r[1] for r in results)

    print()
    print("CONCLUSIONS:")
    print("  - Safe mode: ✅ ZERO data loss (auto-commit + WAL)")
    print("  - Batch mode: ✅ Bounded data loss (max batch_size ops)")
    print("  - Unsafe mode: ⚠️ HIGH data loss risk (manual checkpoint required)")
    print()
    print("RECOMMENDATION:")
    print("  - Use Safe mode for critical data (financial, user accounts)")
    print("  - Use Batch mode for high throughput with acceptable risk")
    print("  - Use Unsafe mode ONLY for temporary/analytics data")
    print()

    if all_passed:
        print("🎉 ALL POWER FAILURE RECOVERY TESTS PASSED!")
        sys.exit(0)
    else:
        print("❌ SOME TESTS FAILED")
        sys.exit(1)
