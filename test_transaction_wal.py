#!/usr/bin/env python3
"""Test WAL with ACTUAL transactions (not just insert_one)"""

import os
import sys
from ironbase import IronBase

def check_wal_size(wal_path):
    """Get WAL file size"""
    if os.path.exists(wal_path):
        return os.path.getsize(wal_path)
    return 0

def test_transaction_wal_growth():
    """Test that WAL grows during transaction"""
    db_path = "test_tx_wal.mlite"
    wal_path = "test_tx_wal.wal"

    # Cleanup
    for f in [db_path, wal_path]:
        if os.path.exists(f):
            os.remove(f)

    print("=== Transaction WAL Growth Test ===\n")

    db = IronBase(db_path)
    col = db.collection("test")

    # Insert some baseline data
    print("Step 1: Insert 10 baseline documents (no transaction)")
    for i in range(10):
        col.insert_one({"value": i, "type": "baseline"})

    wal_baseline = check_wal_size(wal_path)
    print(f"  WAL size after baseline inserts: {wal_baseline} bytes")
    print(f"  (Should be 0 - no transaction used)")

    # Start transaction
    print("\nStep 2: Begin transaction")
    tx_id = db.begin_transaction()
    print(f"  Transaction ID: {tx_id}")

    wal_after_begin = check_wal_size(wal_path)
    print(f"  WAL size after BEGIN: {wal_after_begin} bytes")

    if wal_after_begin > 0:
        print("  ✓ WAL contains BEGIN marker!")
    else:
        print("  ✗ WAL still empty after BEGIN")

    # Add operations to transaction
    print("\nStep 3: Add operations to transaction")

    # Insert operation
    doc_id1 = col.insert_one({"value": 100, "type": "transaction"})
    wal_after_op1 = check_wal_size(wal_path)
    print(f"  WAL after insert op: {wal_after_op1} bytes")

    # Update operation
    col.update_one({"_id": doc_id1}, {"$set": {"updated": True}})
    wal_after_op2 = check_wal_size(wal_path)
    print(f"  WAL after update op: {wal_after_op2} bytes")

    # Delete operation
    col.delete_one({"value": 0})
    wal_after_op3 = check_wal_size(wal_path)
    print(f"  WAL after delete op: {wal_after_op3} bytes")

    if wal_after_op3 > wal_after_op2 > wal_after_op1 > wal_after_begin:
        print("  ✓ WAL growing with each operation!")
    else:
        print(f"  ⚠ WAL not growing as expected:")
        print(f"    BEGIN: {wal_after_begin}")
        print(f"    +insert: {wal_after_op1}")
        print(f"    +update: {wal_after_op2}")
        print(f"    +delete: {wal_after_op3}")

    # Commit transaction
    print("\nStep 4: Commit transaction")
    db.commit_transaction(tx_id)

    wal_after_commit = check_wal_size(wal_path)
    print(f"  WAL size after COMMIT: {wal_after_commit} bytes")

    if wal_after_commit == 0:
        print("  ✓ WAL cleared after successful commit!")
    else:
        print(f"  ⚠ WAL not cleared ({wal_after_commit} bytes remain)")

    # Verify data
    print("\nStep 5: Verify data integrity")
    count_total = col.count_documents({})
    count_baseline = col.count_documents({"type": "baseline"})
    count_tx = col.count_documents({"type": "transaction"})

    print(f"  Total documents: {count_total}")
    print(f"  Baseline documents: {count_baseline}")
    print(f"  Transaction documents: {count_tx}")

    expected_baseline = 9  # 10 - 1 deleted
    expected_tx = 1

    if count_baseline == expected_baseline and count_tx == expected_tx:
        print(f"  ✓ Data correct: {expected_baseline} baseline + {expected_tx} transaction")
    else:
        print(f"  ✗ Data incorrect: expected {expected_baseline}+{expected_tx}, got {count_baseline}+{count_tx}")
        return False

    db.close()

    # Cleanup
    for f in [db_path, wal_path]:
        if os.path.exists(f):
            os.remove(f)

    return True


def test_transaction_crash_recovery():
    """Test crash recovery with uncommitted transaction"""
    db_path = "test_tx_crash.mlite"
    wal_path = "test_tx_crash.wal"

    # Cleanup
    for f in [db_path, wal_path]:
        if os.path.exists(f):
            os.remove(f)

    print("\n\n=== Transaction Crash Recovery Test ===\n")

    # Phase 1: Start transaction but DON'T commit
    print("Phase 1: Start transaction WITHOUT commit")
    db = IronBase(db_path)
    col = db.collection("test")

    # Baseline data
    col.insert_one({"value": 1, "committed": True})
    col.insert_one({"value": 2, "committed": True})

    # Start transaction
    tx_id = db.begin_transaction()
    print(f"  Transaction ID: {tx_id}")

    # Add operations (should go to WAL)
    col.insert_one({"value": 100, "committed": False})
    col.insert_one({"value": 200, "committed": False})

    wal_size = check_wal_size(wal_path)
    print(f"  WAL size with uncommitted tx: {wal_size} bytes")

    if wal_size > 0:
        print("  ✓ WAL contains uncommitted transaction")
    else:
        print("  ✗ WAL empty (transaction not in WAL?)")

    # Simulate crash - DON'T commit, DON'T close
    print("\n  💥 SIMULATED CRASH (uncommitted transaction)")
    del col
    del db

    # Phase 2: Reopen and check recovery
    print("\nPhase 2: Reopen database")
    db2 = IronBase(db_path)
    col2 = db2.collection("test")

    wal_after_reopen = check_wal_size(wal_path)
    print(f"  WAL size after reopen: {wal_after_reopen} bytes")

    count_committed = col2.count_documents({"committed": True})
    count_uncommitted = col2.count_documents({"committed": False})

    print(f"  Committed documents: {count_committed}")
    print(f"  Uncommitted documents: {count_uncommitted}")

    # Uncommitted transaction should be discarded
    if count_committed == 2 and count_uncommitted == 0:
        print("  ✓ Uncommitted transaction discarded (correct!)")
    else:
        print(f"  ✗ Expected 2 committed + 0 uncommitted, got {count_committed} + {count_uncommitted}")
        return False

    if wal_after_reopen == 0:
        print("  ✓ WAL cleared after recovery")

    db2.close()

    # Cleanup
    for f in [db_path, wal_path]:
        if os.path.exists(f):
            os.remove(f)

    return True


def test_transaction_commit_recovery():
    """Test recovery of COMMITTED transaction"""
    db_path = "test_tx_commit.mlite"
    wal_path = "test_tx_commit.wal"

    # Cleanup
    for f in [db_path, wal_path]:
        if os.path.exists(f):
            os.remove(f)

    print("\n\n=== Committed Transaction Recovery Test ===\n")

    # Phase 1: Commit transaction, crash before flush
    print("Phase 1: Commit transaction, crash before final flush")
    db = IronBase(db_path)
    col = db.collection("test")

    # Start and commit transaction
    tx_id = db.begin_transaction()
    col.insert_one({"value": 100, "status": "committed"})
    col.insert_one({"value": 200, "status": "committed"})
    db.commit_transaction(tx_id)

    print(f"  Transaction committed (ID: {tx_id})")

    wal_after_commit = check_wal_size(wal_path)
    print(f"  WAL size after commit: {wal_after_commit} bytes")

    # Don't close - simulate crash
    print("\n  💥 SIMULATED CRASH (after commit, before full flush)")
    del col
    del db

    # Phase 2: Recovery
    print("\nPhase 2: Reopen and verify committed data")
    db2 = IronBase(db_path)
    col2 = db2.collection("test")

    count = col2.count_documents({"status": "committed"})
    print(f"  Committed documents after recovery: {count}")

    if count == 2:
        print("  ✓ Committed transaction data recovered!")
    else:
        print(f"  ✗ Expected 2 documents, got {count}")
        return False

    db2.close()

    # Cleanup
    for f in [db_path, wal_path]:
        if os.path.exists(f):
            os.remove(f)

    return True


if __name__ == "__main__":
    print("=" * 60)
    print("  TRANSACTION WAL TESTS")
    print("=" * 60)
    print()

    results = []

    # Run all tests
    results.append(("Transaction WAL growth", test_transaction_wal_growth()))
    results.append(("Uncommitted transaction crash", test_transaction_crash_recovery()))
    results.append(("Committed transaction recovery", test_transaction_commit_recovery()))

    # Summary
    print("\n" + "=" * 60)
    print("  SUMMARY")
    print("=" * 60)

    for name, passed in results:
        status = "✓ PASS" if passed else "✗ FAIL"
        print(f"  {status}: {name}")

    all_passed = all(r[1] for r in results)

    print()
    if all_passed:
        print("🎉 ALL TRANSACTION WAL TESTS PASSED!")
        sys.exit(0)
    else:
        print("❌ SOME TESTS FAILED")
        sys.exit(1)
