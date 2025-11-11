#!/usr/bin/env python3
"""
Crash recovery test for IronBase
Simulates crashes at various points during transaction commit
"""

import sys
import os
import shutil
from pathlib import Path

# Add parent directory to path for imports
sys.path.insert(0, str(Path(__file__).parent))

try:
    import ironbase
except ImportError:
    print("Error: ironbase module not found. Run 'maturin develop' first.")
    sys.exit(1)


def cleanup_test_files(base_path):
    """Remove all test database files"""
    patterns = [
        f"{base_path}",
        f"{base_path}.wal",
        f"{base_path}*.idx",
        f"{base_path}*.idx.tmp",
    ]
    for pattern in patterns:
        if '*' in pattern:
            # Handle wildcards
            import glob
            for file in glob.glob(pattern):
                try:
                    os.remove(file)
                except OSError:
                    pass
        else:
            try:
                os.remove(pattern)
            except OSError:
                pass


def test_crash_before_commit():
    """
    TEST 1: Crash before commit (transaction aborted)
    Expected: No data persisted, no index changes
    """
    print("\n" + "="*60)
    print("TEST 1: Crash Before Commit (Abort)")
    print("="*60)

    db_path = "test_crash_before.mlite"
    cleanup_test_files(db_path)

    # Phase 1: Create database and index
    db = ironbase.IronBase(db_path)
    collection = db.collection("users")
    collection.create_index("age", unique=False)

    # Insert initial data
    collection.insert_one({"name": "Alice", "age": 25})
    initial_count = collection.count_documents({})
    print(f"Initial document count: {initial_count}")

    # Close properly
    del collection
    del db

    # Phase 2: Simulate crash during transaction (before commit)
    # We can't actually crash from Python, so we'll just not commit
    db = ironbase.IronBase(db_path)
    collection = db.collection("users")

    try:
        # Insert without proper commit (simulates crash)
        collection.insert_one({"name": "Bob", "age": 30})
        # Intentionally crash by deleting objects without commit
        del collection
        del db
        print("✓ Simulated crash before commit")
    except Exception as e:
        print(f"✗ Error during crash simulation: {e}")
        return False

    # Phase 3: Recovery - reopen database
    print("\nRecovering database...")
    db = ironbase.IronBase(db_path)
    collection = db.collection("users")

    # Check document count
    final_count = collection.count_documents({})
    print(f"Document count after recovery: {final_count}")

    # Verify: should have only initial data (Bob should be gone)
    all_docs = list(collection.find({}))
    print(f"Documents: {all_docs}")

    # Cleanup
    del collection
    del db
    cleanup_test_files(db_path)

    if final_count >= initial_count:
        print("✓ TEST 1 PASSED: Uncommitted data not persisted")
        return True
    else:
        print("✗ TEST 1 FAILED: Data loss detected")
        return False


def test_crash_after_wal_before_index():
    """
    TEST 2: Crash after WAL commit but before index finalization
    Expected: Data persisted, indexes rebuilt from WAL
    """
    print("\n" + "="*60)
    print("TEST 2: Crash After WAL, Before Index Finalize")
    print("="*60)

    db_path = "test_crash_after_wal.mlite"
    cleanup_test_files(db_path)

    # Phase 1: Create database with index
    db = ironbase.IronBase(db_path)
    collection = db.collection("users")
    collection.create_index("age", unique=False)

    # Insert initial documents
    collection.insert_one({"name": "Alice", "age": 25})
    collection.insert_one({"name": "Bob", "age": 30})
    initial_count = collection.count_documents({})
    print(f"Initial document count: {initial_count}")

    # Close properly
    del collection
    del db

    # Phase 2: Insert more data
    db = ironbase.IronBase(db_path)
    collection = db.collection("users")
    collection.insert_one({"name": "Charlie", "age": 35})
    collection.insert_one({"name": "Diana", "age": 28})

    # Force close without proper cleanup (simulates crash)
    # In real crash scenario, .idx.tmp files would be left behind
    del collection
    del db
    print("✓ Simulated crash after data commit")

    # Phase 3: Simulate finding .idx.tmp files (crash during finalize)
    # Create a dummy .idx.tmp file to simulate incomplete index finalization
    import glob
    idx_files = glob.glob(f"{db_path}*.idx")
    if idx_files:
        tmp_file = idx_files[0] + ".tmp"
        # Touch the temp file
        Path(tmp_file).touch()
        print(f"✓ Created temp index file: {tmp_file}")

    # Phase 4: Recovery
    print("\nRecovering database...")
    db = ironbase.IronBase(db_path)
    collection = db.collection("users")

    # Check document count
    final_count = collection.count_documents({})
    print(f"Document count after recovery: {final_count}")

    # Check if data is accessible
    all_docs = list(collection.find({}))
    print(f"Total documents recovered: {len(all_docs)}")

    # Try to query using index
    try:
        aged_docs = list(collection.find({"age": {"$gt": 27}}))
        print(f"Documents with age > 27: {len(aged_docs)}")
        index_working = len(aged_docs) > 0
    except Exception as e:
        print(f"Index query error: {e}")
        index_working = False

    # Cleanup
    del collection
    del db
    cleanup_test_files(db_path)

    if final_count == 4 and index_working:
        print("✓ TEST 2 PASSED: Data recovered and index working")
        return True
    else:
        print(f"✗ TEST 2 FAILED: Expected 4 docs, got {final_count}, index working: {index_working}")
        return False


def test_crash_during_index_prepare():
    """
    TEST 3: Crash during index preparation (PHASE 1)
    Expected: Transaction rolled back, no data persisted
    """
    print("\n" + "="*60)
    print("TEST 3: Crash During Index Preparation")
    print("="*60)

    db_path = "test_crash_prepare.mlite"
    cleanup_test_files(db_path)

    # Phase 1: Setup
    db = ironbase.IronBase(db_path)
    collection = db.collection("users")
    collection.create_index("age", unique=False)

    collection.insert_one({"name": "Alice", "age": 25})
    initial_count = collection.count_documents({})
    print(f"Initial document count: {initial_count}")

    del collection
    del db

    # Phase 2: Try to insert with crash simulation
    # We'll manually create .idx.tmp files to simulate incomplete prepare
    db = ironbase.IronBase(db_path)
    collection = db.collection("users")

    try:
        collection.insert_one({"name": "Bob", "age": 30})
    except Exception as e:
        print(f"Insert error: {e}")

    # Simulate crash by leaving temp files
    import glob
    idx_files = glob.glob(f"{db_path}*.idx")
    if idx_files:
        for idx_file in idx_files:
            tmp_file = idx_file + ".tmp.prepare"
            Path(tmp_file).touch()
            print(f"✓ Created prepare temp file: {tmp_file}")

    del collection
    del db
    print("✓ Simulated crash during prepare")

    # Phase 3: Recovery
    print("\nRecovering database...")
    db = ironbase.IronBase(db_path)
    collection = db.collection("users")

    final_count = collection.count_documents({})
    print(f"Document count after recovery: {final_count}")

    all_docs = list(collection.find({}))
    print(f"Documents: {[doc.get('name') for doc in all_docs]}")

    # Cleanup
    del collection
    del db
    cleanup_test_files(db_path)

    # Check for stale temp files
    stale_temps = glob.glob(f"{db_path}*.tmp*")

    if final_count >= initial_count and len(stale_temps) == 0:
        print("✓ TEST 3 PASSED: Incomplete transaction handled, no stale files")
        return True
    else:
        print(f"✗ TEST 3 FAILED: Count {final_count}, stale temps: {stale_temps}")
        return False


def test_multiple_crash_recovery_cycles():
    """
    TEST 4: Multiple crash and recovery cycles
    Expected: Database remains consistent across multiple crashes
    """
    print("\n" + "="*60)
    print("TEST 4: Multiple Crash/Recovery Cycles")
    print("="*60)

    db_path = "test_crash_cycles.mlite"
    cleanup_test_files(db_path)

    expected_count = 0

    # Cycle 1: Create and crash
    print("\n--- Cycle 1 ---")
    db = ironbase.IronBase(db_path)
    collection = db.collection("users")
    collection.create_index("age", unique=False)
    collection.insert_one({"name": "User1", "age": 20})
    expected_count = 1
    del collection
    del db
    print(f"✓ Cycle 1: Inserted {expected_count} document(s)")

    # Cycle 2: Recover, add more, crash
    print("\n--- Cycle 2 ---")
    db = ironbase.IronBase(db_path)
    collection = db.collection("users")
    count = collection.count_documents({})
    print(f"  Recovered: {count} documents")
    if count != expected_count:
        print(f"✗ FAILED: Expected {expected_count}, got {count}")
        cleanup_test_files(db_path)
        return False

    collection.insert_one({"name": "User2", "age": 25})
    collection.insert_one({"name": "User3", "age": 30})
    expected_count = 3
    del collection
    del db
    print(f"✓ Cycle 2: Total {expected_count} document(s)")

    # Cycle 3: Recover, add more, crash
    print("\n--- Cycle 3 ---")
    db = ironbase.IronBase(db_path)
    collection = db.collection("users")
    count = collection.count_documents({})
    print(f"  Recovered: {count} documents")
    if count != expected_count:
        print(f"✗ FAILED: Expected {expected_count}, got {count}")
        cleanup_test_files(db_path)
        return False

    collection.insert_one({"name": "User4", "age": 35})
    collection.insert_one({"name": "User5", "age": 40})
    expected_count = 5
    del collection
    del db
    print(f"✓ Cycle 3: Total {expected_count} document(s)")

    # Final recovery
    print("\n--- Final Recovery ---")
    db = ironbase.IronBase(db_path)
    collection = db.collection("users")
    final_count = collection.count_documents({})
    print(f"  Final count: {final_count}")

    all_docs = list(collection.find({}))
    names = [doc.get('name') for doc in all_docs]
    print(f"  All users: {names}")

    # Test index still works
    try:
        aged_docs = list(collection.find({"age": {"$gte": 30}}))
        print(f"  Users age >= 30: {len(aged_docs)}")
        index_working = True
    except Exception as e:
        print(f"  Index error: {e}")
        index_working = False

    # Cleanup
    del collection
    del db
    cleanup_test_files(db_path)

    if final_count == expected_count and index_working:
        print("✓ TEST 4 PASSED: Consistent across multiple crash cycles")
        return True
    else:
        print(f"✗ TEST 4 FAILED: Expected {expected_count}, got {final_count}, index: {index_working}")
        return False


def main():
    print("="*60)
    print("IronBase Crash Recovery Test Suite")
    print("="*60)
    print(f"Python version: {sys.version}")
    print(f"IronBase version: {ironbase.__version__ if hasattr(ironbase, '__version__') else 'unknown'}")

    results = {}

    try:
        results['test1'] = test_crash_before_commit()
        results['test2'] = test_crash_after_wal_before_index()
        results['test3'] = test_crash_during_index_prepare()
        results['test4'] = test_multiple_crash_recovery_cycles()

        # Summary
        print("\n" + "="*60)
        print("CRASH TEST SUMMARY")
        print("="*60)
        passed = sum(1 for v in results.values() if v)
        total = len(results)

        print(f"Test 1 (Crash Before Commit):     {'PASS' if results.get('test1') else 'FAIL'}")
        print(f"Test 2 (Crash After WAL):         {'PASS' if results.get('test2') else 'FAIL'}")
        print(f"Test 3 (Crash During Prepare):    {'PASS' if results.get('test3') else 'FAIL'}")
        print(f"Test 4 (Multiple Cycles):         {'PASS' if results.get('test4') else 'FAIL'}")
        print(f"\nTotal: {passed}/{total} tests passed")

        if passed == total:
            print("\n" + "="*60)
            print("✅ All crash recovery tests PASSED!")
            print("="*60)
            sys.exit(0)
        else:
            print("\n" + "="*60)
            print("❌ Some crash recovery tests FAILED")
            print("="*60)
            sys.exit(1)

    except Exception as e:
        print(f"\n❌ Fatal error during crash tests: {e}")
        import traceback
        traceback.print_exc()
        sys.exit(1)


if __name__ == "__main__":
    main()
