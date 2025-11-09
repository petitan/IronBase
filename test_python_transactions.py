#!/usr/bin/env python3
"""Test Python transaction bindings for MongoLite ACD transactions"""

import mongolite
import tempfile
import os

def test_basic_transaction():
    """Test basic transaction commit"""
    with tempfile.TemporaryDirectory() as tmpdir:
        db_path = os.path.join(tmpdir, "test.mlite")
        db = mongolite.MongoLite(db_path)

        # Begin transaction
        tx_id = db.begin_transaction()
        print(f"✓ Started transaction {tx_id}")

        # Insert document in transaction
        result = db.insert_one_tx("users", {"name": "Alice", "age": 30}, tx_id)
        print(f"✓ Inserted document: {result}")
        assert result["acknowledged"] == True
        assert "inserted_id" in result

        # Commit transaction
        db.commit_transaction(tx_id)
        print(f"✓ Committed transaction {tx_id}")

        # Verify data was committed
        users = db.collection("users")
        doc = users.find_one({"name": "Alice"})
        assert doc is not None
        assert doc["name"] == "Alice"
        assert doc["age"] == 30
        print(f"✓ Data persisted: {doc}")


def test_transaction_rollback():
    """Test transaction rollback"""
    with tempfile.TemporaryDirectory() as tmpdir:
        db_path = os.path.join(tmpdir, "test.mlite")
        db = mongolite.MongoLite(db_path)

        # Insert initial data
        users = db.collection("users")
        users.insert_one({"name": "Bob", "age": 25})

        # Begin transaction
        tx_id = db.begin_transaction()
        print(f"✓ Started transaction {tx_id}")

        # Insert in transaction
        db.insert_one_tx("users", {"name": "Charlie", "age": 35}, tx_id)
        print(f"✓ Inserted Charlie in transaction")

        # Rollback transaction
        db.rollback_transaction(tx_id)
        print(f"✓ Rolled back transaction {tx_id}")

        # Verify Charlie was not persisted
        doc = users.find_one({"name": "Charlie"})
        assert doc is None
        print(f"✓ Charlie not found (rollback worked)")

        # Verify Bob is still there
        doc = users.find_one({"name": "Bob"})
        assert doc is not None
        assert doc["age"] == 25
        print(f"✓ Bob still exists: {doc}")


def test_multi_operation_transaction():
    """Test transaction with multiple operations"""
    with tempfile.TemporaryDirectory() as tmpdir:
        db_path = os.path.join(tmpdir, "test.mlite")
        db = mongolite.MongoLite(db_path)

        # Insert initial data
        users = db.collection("users")
        users.insert_one({"name": "Alice", "age": 30})
        users.insert_one({"name": "Bob", "age": 25})

        # Begin transaction
        tx_id = db.begin_transaction()
        print(f"✓ Started transaction {tx_id}")

        # Insert
        db.insert_one_tx("users", {"name": "Charlie", "age": 35}, tx_id)
        print(f"✓ Inserted Charlie")

        # Update
        result = db.update_one_tx("users", {"name": "Alice"}, {"name": "Alice", "age": 31}, tx_id)
        print(f"✓ Updated Alice: {result}")
        assert result["matched_count"] == 1
        assert result["modified_count"] == 1

        # Delete
        result = db.delete_one_tx("users", {"name": "Bob"}, tx_id)
        print(f"✓ Deleted Bob: {result}")
        assert result["deleted_count"] == 1

        # Commit
        db.commit_transaction(tx_id)
        print(f"✓ Committed transaction {tx_id}")

        # Verify all changes
        alice = users.find_one({"name": "Alice"})
        assert alice["age"] == 31
        print(f"✓ Alice updated to age 31")

        bob = users.find_one({"name": "Bob"})
        assert bob is None
        print(f"✓ Bob deleted")

        charlie = users.find_one({"name": "Charlie"})
        assert charlie is not None
        assert charlie["age"] == 35
        print(f"✓ Charlie inserted")


def test_multi_collection_transaction():
    """Test transaction across multiple collections"""
    with tempfile.TemporaryDirectory() as tmpdir:
        db_path = os.path.join(tmpdir, "test.mlite")
        db = mongolite.MongoLite(db_path)

        # Begin transaction
        tx_id = db.begin_transaction()
        print(f"✓ Started transaction {tx_id}")

        # Insert into multiple collections
        db.insert_one_tx("users", {"name": "Alice", "role": "admin"}, tx_id)
        db.insert_one_tx("logs", {"action": "user_created", "user": "Alice"}, tx_id)
        db.insert_one_tx("settings", {"key": "max_users", "value": 100}, tx_id)
        print(f"✓ Inserted into 3 collections")

        # Commit
        db.commit_transaction(tx_id)
        print(f"✓ Committed transaction {tx_id}")

        # Verify data in all collections
        users = db.collection("users")
        logs = db.collection("logs")
        settings = db.collection("settings")

        assert users.find_one({"name": "Alice"}) is not None
        assert logs.find_one({"action": "user_created"}) is not None
        assert settings.find_one({"key": "max_users"}) is not None
        print(f"✓ All collections updated atomically")


def test_transaction_not_found():
    """Test error handling for invalid transaction ID"""
    with tempfile.TemporaryDirectory() as tmpdir:
        db_path = os.path.join(tmpdir, "test.mlite")
        db = mongolite.MongoLite(db_path)

        try:
            db.insert_one_tx("users", {"name": "Alice"}, 9999)
            assert False, "Should have raised exception"
        except Exception as e:
            assert "Transaction 9999 not found" in str(e)
            print(f"✓ Correctly raised error for invalid transaction ID")


if __name__ == "__main__":
    print("=== Testing Python Transaction Bindings ===\n")

    print("Test 1: Basic transaction commit")
    test_basic_transaction()
    print()

    print("Test 2: Transaction rollback")
    test_transaction_rollback()
    print()

    print("Test 3: Multi-operation transaction")
    test_multi_operation_transaction()
    print()

    print("Test 4: Multi-collection transaction")
    test_multi_collection_transaction()
    print()

    print("Test 5: Transaction not found error")
    test_transaction_not_found()
    print()

    print("=== All tests passed! ✅ ===")
