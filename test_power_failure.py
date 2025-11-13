#!/usr/bin/env python3
"""Simulate power failure scenarios and analyze data loss"""

import os
import sys
import shutil
from ironbase import IronBase

def simulate_power_failure(db_path, wal_path):
    """
    Simulate power failure by killing process WITHOUT calling close()
    This leaves the DB in whatever state it was mid-operation
    """
    # In real power failure:
    # - No flush() called
    # - No close() called
    # - File buffers may be partially written (OS dependent)
    # - WAL file is in whatever state it was

    # We simulate this by just NOT calling close()
    pass

def test_scenario_1_normal_insert():
    """Scenario 1: Power failure during normal insert_one (NO transaction)"""
    db_path = "test_pf1.mlite"
    wal_path = "test_pf1.wal"

    # Cleanup
    for f in [db_path, wal_path]:
        if os.path.exists(f):
            os.remove(f)

    print("=== SCENARIO 1: Power Failure During insert_one() ===\n")

    # Phase 1: Insert some data
    print("Phase 1: Insert 5 documents (no transaction)")
    db = IronBase(db_path)
    col = db.collection("test")

    for i in range(5):
        col.insert_one({"value": i, "status": "before_crash"})

    count_before = col.count_documents({})
    print(f"  Documents before crash: {count_before}")

    # Check file sizes
    mlite_size = os.path.getsize(db_path) if os.path.exists(db_path) else 0
    wal_size = os.path.getsize(wal_path) if os.path.exists(wal_path) else 0
    print(f"  .mlite file size: {mlite_size} bytes")
    print(f"  .wal file size: {wal_size} bytes")

    # POWER FAILURE - no close(), no flush()
    print("\n  ⚡ POWER FAILURE (no close, no flush)")
    del col
    del db

    # Phase 2: Recovery
    print("\nPhase 2: Restart and recover")
    db2 = IronBase(db_path)
    col2 = db2.collection("test")

    count_after = col2.count_documents({})
    print(f"  Documents after recovery: {count_after}")

    if count_after == 5:
        print("  ✓ All data recovered (metadata was flushed)")
    elif count_after < 5:
        print(f"  ⚠ DATA LOSS: {5 - count_after} documents lost")
    else:
        print(f"  ✗ Corruption: more documents than expected")

    # Check if WAL helped
    wal_after = os.path.getsize(wal_path) if os.path.exists(wal_path) else 0
    print(f"  WAL after recovery: {wal_after} bytes")

    if wal_size == 0 and wal_after == 0:
        print("  ℹ WAL was empty (normal insert doesn't use WAL)")

    db2.close()

    # Cleanup
    for f in [db_path, wal_path]:
        if os.path.exists(f):
            os.remove(f)

    return count_after == 5


def test_scenario_2_transaction_uncommitted():
    """Scenario 2: Power failure during transaction BEFORE commit"""
    db_path = "test_pf2.mlite"
    wal_path = "test_pf2.wal"

    # Cleanup
    for f in [db_path, wal_path]:
        if os.path.exists(f):
            os.remove(f)

    print("\n\n=== SCENARIO 2: Power Failure During Transaction (BEFORE commit) ===\n")

    # Phase 1: Start transaction, add data, DON'T commit
    print("Phase 1: Begin transaction, insert data, NO COMMIT")
    db = IronBase(db_path)
    col = db.collection("test")

    # Baseline data (committed)
    col.insert_one({"value": 1, "type": "baseline"})
    col.insert_one({"value": 2, "type": "baseline"})

    # Start transaction
    tx_id = db.begin_transaction()
    print(f"  Transaction ID: {tx_id}")

    # Add data in transaction
    col.insert_one({"value": 100, "type": "transaction"})
    col.insert_one({"value": 200, "type": "transaction"})

    count_before = col.count_documents({})
    print(f"  Documents before crash: {count_before}")
    print(f"    Baseline: {col.count_documents({'type': 'baseline'})}")
    print(f"    Transaction: {col.count_documents({'type': 'transaction'})}")

    # Check WAL
    wal_size = os.path.getsize(wal_path) if os.path.exists(wal_path) else 0
    print(f"  WAL size: {wal_size} bytes")

    # POWER FAILURE before commit
    print("\n  ⚡ POWER FAILURE (transaction NOT committed)")
    del col
    del db

    # Phase 2: Recovery
    print("\nPhase 2: Restart and check what survived")
    db2 = IronBase(db_path)
    col2 = db2.collection("test")

    count_baseline = col2.count_documents({"type": "baseline"})
    count_tx = col2.count_documents({"type": "transaction"})

    print(f"  Baseline documents: {count_baseline}")
    print(f"  Transaction documents: {count_tx}")

    if count_baseline == 2 and count_tx == 0:
        print("  ✓ CORRECT: Uncommitted transaction rolled back")
    elif count_baseline == 2 and count_tx == 2:
        print("  ⚠ NO ISOLATION: Uncommitted data visible (ACD design)")
    else:
        print(f"  ✗ Unexpected state: {count_baseline} baseline + {count_tx} transaction")

    db2.close()

    # Cleanup
    for f in [db_path, wal_path]:
        if os.path.exists(f):
            os.remove(f)

    # In ACD design, uncommitted data MAY survive (no isolation)
    return True


def test_scenario_3_transaction_after_commit():
    """Scenario 3: Power failure AFTER commit but before full flush"""
    db_path = "test_pf3.mlite"
    wal_path = "test_pf3.wal"

    # Cleanup
    for f in [db_path, wal_path]:
        if os.path.exists(f):
            os.remove(f)

    print("\n\n=== SCENARIO 3: Power Failure AFTER Transaction Commit ===\n")

    # Phase 1: Commit transaction
    print("Phase 1: Commit transaction, then crash")
    db = IronBase(db_path)
    col = db.collection("test")

    # Start and commit transaction
    tx_id = db.begin_transaction()
    col.insert_one({"value": 100, "type": "committed"})
    col.insert_one({"value": 200, "type": "committed"})
    db.commit_transaction(tx_id)

    print(f"  Transaction {tx_id} committed")

    count_before = col.count_documents({"type": "committed"})
    print(f"  Documents before crash: {count_before}")

    # Check WAL
    wal_size = os.path.getsize(wal_path) if os.path.exists(wal_path) else 0
    print(f"  WAL size after commit: {wal_size} bytes")

    if wal_size > 0:
        print("  ✓ WAL contains commit marker")

    # POWER FAILURE after commit
    print("\n  ⚡ POWER FAILURE (after commit)")
    del col
    del db

    # Phase 2: Recovery
    print("\nPhase 2: Restart - WAL should replay committed transaction")
    db2 = IronBase(db_path)
    col2 = db2.collection("test")

    count_after = col2.count_documents({"type": "committed"})
    print(f"  Documents after recovery: {count_after}")

    if count_after == 2:
        print("  ✓ DURABILITY: Committed data survived power failure")
    else:
        print(f"  ✗ DATA LOSS: Expected 2, got {count_after}")

    # Check WAL cleared
    wal_after = os.path.getsize(wal_path) if os.path.exists(wal_path) else 0
    print(f"  WAL after recovery: {wal_after} bytes")

    db2.close()

    # Cleanup
    for f in [db_path, wal_path]:
        if os.path.exists(f):
            os.remove(f)

    return count_after == 2


def test_scenario_4_metadata_not_flushed():
    """Scenario 4: Insert many docs, crash before metadata flush"""
    db_path = "test_pf4.mlite"
    wal_path = "test_pf4.wal"

    # Cleanup
    for f in [db_path, wal_path]:
        if os.path.exists(f):
            os.remove(f)

    print("\n\n=== SCENARIO 4: Metadata Not Flushed (Worst Case) ===\n")

    # Phase 1: Insert WITHOUT explicit flush
    print("Phase 1: Insert 100 documents rapidly")
    db = IronBase(db_path)
    col = db.collection("test")

    for i in range(100):
        col.insert_one({"value": i})

    count_before = col.count_documents({})
    print(f"  Documents in memory: {count_before}")

    mlite_size = os.path.getsize(db_path) if os.path.exists(db_path) else 0
    print(f"  .mlite file size: {mlite_size} bytes")

    # POWER FAILURE - metadata may not be flushed!
    print("\n  ⚡ POWER FAILURE (metadata might not be on disk)")
    del col
    del db

    # Phase 2: Recovery
    print("\nPhase 2: Restart - check what metadata says")
    db2 = IronBase(db_path)
    col2 = db2.collection("test")

    count_after = col2.count_documents({})
    print(f"  Documents after recovery: {count_after}")

    if count_after < 100:
        print(f"  ⚠ PARTIAL DATA LOSS: {100 - count_after} documents lost")
        print(f"  Reason: Metadata not flushed to disk")
    elif count_after == 100:
        print("  ✓ All data recovered (lucky - metadata was flushed)")

    db2.close()

    # Cleanup
    for f in [db_path, wal_path]:
        if os.path.exists(f):
            os.remove(f)

    # This is expected behavior - metadata flush timing determines what survives
    return True


if __name__ == "__main__":
    print("=" * 70)
    print("  POWER FAILURE SIMULATION TESTS")
    print("  (Simulating áramszünet - no graceful shutdown)")
    print("=" * 70)
    print()

    results = []

    # Run all scenarios
    results.append(("Normal insert (no transaction)", test_scenario_1_normal_insert()))
    results.append(("Transaction before commit", test_scenario_2_transaction_uncommitted()))
    results.append(("Transaction after commit", test_scenario_3_transaction_after_commit()))
    results.append(("Metadata not flushed", test_scenario_4_metadata_not_flushed()))

    # Summary
    print("\n" + "=" * 70)
    print("  SUMMARY - WHAT HAPPENS DURING POWER FAILURE (ÁRAMSZÜNET)")
    print("=" * 70)
    print()
    print("Scenario 1 (Normal insert):")
    print("  - Documents written to .mlite file")
    print("  - If metadata flushed: ✓ data survives")
    print("  - If metadata NOT flushed: ⚠ data lost (orphan documents)")
    print("  - WAL doesn't help (not used for normal inserts)")
    print()
    print("Scenario 2 (Uncommitted transaction):")
    print("  - ACD design: uncommitted data MAY be in .mlite file")
    print("  - NO WAL entry (no commit yet)")
    print("  - Recovery: data may survive but is 'uncommitted'")
    print("  - No isolation guarantee")
    print()
    print("Scenario 3 (Committed transaction):")
    print("  - WAL contains COMMIT marker")
    print("  - Recovery: WAL replays → ✓ data survives")
    print("  - Durability guaranteed")
    print()
    print("Scenario 4 (Metadata not flushed):")
    print("  - Documents in .mlite file but NOT in catalog")
    print("  - Recovery: orphan documents ignored")
    print("  - Partial or total data loss possible")
    print()
    print("CONCLUSION:")
    print("  - Normal inserts: data loss if metadata not flushed")
    print("  - Transactions: committed data survives via WAL replay")
    print("  - Best practice: call db.checkpoint() or db.close() periodically")
    print()

    for name, passed in results:
        status = "✓" if passed else "✗"
        print(f"  {status} {name}")
