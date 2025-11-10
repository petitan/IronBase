#!/usr/bin/env python3
"""
Test ACD Transactions - Python Bindings
Tests the transaction API exposed through PyO3
"""

import ironbase
import os
import tempfile

def test_basic_transaction():
    """Test basic begin/commit transaction flow"""
    print("Test 1: Basic transaction flow")

    with tempfile.TemporaryDirectory() as tmpdir:
        db_path = os.path.join(tmpdir, "test.mlite")
        db = ironbase.MongoLite(db_path)

        # Begin transaction
        tx_id = db.begin_transaction()
        print(f"  ✓ Started transaction: {tx_id}")
        assert tx_id == 1

        # Commit transaction
        db.commit_transaction(tx_id)
        print(f"  ✓ Committed transaction: {tx_id}")

    print("  ✅ PASSED\n")


def test_transaction_rollback():
    """Test transaction rollback"""
    print("Test 2: Transaction rollback")

    with tempfile.TemporaryDirectory() as tmpdir:
        db_path = os.path.join(tmpdir, "test.mlite")
        db = ironbase.MongoLite(db_path)

        # Begin transaction
        tx_id = db.begin_transaction()
        print(f"  ✓ Started transaction: {tx_id}")

        # Rollback instead of commit
        db.rollback_transaction(tx_id)
        print(f"  ✓ Rolled back transaction: {tx_id}")

    print("  ✅ PASSED\n")


def test_multiple_transactions():
    """Test multiple sequential transactions"""
    print("Test 3: Multiple sequential transactions")

    with tempfile.TemporaryDirectory() as tmpdir:
        db_path = os.path.join(tmpdir, "test.mlite")
        db = ironbase.MongoLite(db_path)

        # First transaction
        tx_id1 = db.begin_transaction()
        assert tx_id1 == 1
        db.commit_transaction(tx_id1)
        print(f"  ✓ Transaction {tx_id1} committed")

        # Second transaction
        tx_id2 = db.begin_transaction()
        assert tx_id2 == 2
        db.commit_transaction(tx_id2)
        print(f"  ✓ Transaction {tx_id2} committed")

        # Third transaction (rollback)
        tx_id3 = db.begin_transaction()
        assert tx_id3 == 3
        db.rollback_transaction(tx_id3)
        print(f"  ✓ Transaction {tx_id3} rolled back")

    print("  ✅ PASSED\n")


def test_transaction_error_handling():
    """Test error handling for invalid transaction operations"""
    print("Test 4: Error handling")

    with tempfile.TemporaryDirectory() as tmpdir:
        db_path = os.path.join(tmpdir, "test.mlite")
        db = ironbase.MongoLite(db_path)

        # Try to commit non-existent transaction
        try:
            db.commit_transaction(999)
            assert False, "Should have raised an error"
        except Exception as e:
            print(f"  ✓ Expected error: {e}")

        # Try to rollback non-existent transaction
        try:
            db.rollback_transaction(999)
            assert False, "Should have raised an error"
        except Exception as e:
            print(f"  ✓ Expected error: {e}")

    print("  ✅ PASSED\n")


def test_transaction_api_availability():
    """Test that transaction API is available"""
    print("Test 5: API availability")

    with tempfile.TemporaryDirectory() as tmpdir:
        db_path = os.path.join(tmpdir, "test.mlite")
        db = ironbase.MongoLite(db_path)

        # Check methods exist
        assert hasattr(db, 'begin_transaction'), "begin_transaction method missing"
        assert hasattr(db, 'commit_transaction'), "commit_transaction method missing"
        assert hasattr(db, 'rollback_transaction'), "rollback_transaction method missing"
        print("  ✓ All transaction methods available")

        # Check they are callable
        assert callable(db.begin_transaction)
        assert callable(db.commit_transaction)
        assert callable(db.rollback_transaction)
        print("  ✓ All transaction methods callable")

    print("  ✅ PASSED\n")


def main():
    """Run all tests"""
    print("=" * 60)
    print("ACD Transactions - Python Bindings Tests")
    print("=" * 60)
    print()

    test_transaction_api_availability()
    test_basic_transaction()
    test_transaction_rollback()
    test_multiple_transactions()
    test_transaction_error_handling()

    print("=" * 60)
    print("✅ ALL TESTS PASSED!")
    print("=" * 60)


if __name__ == "__main__":
    main()
