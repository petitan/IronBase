#!/usr/bin/env python3
"""
MongoLite ACD Transactions - Python Example

This example demonstrates how to use ACD (Atomicity, Consistency, Durability)
transactions in MongoLite from Python.

ACD provides reliable transaction support without the complexity of full ACID:
- Atomicity: All operations succeed or fail together
- Consistency: Data integrity is maintained
- Durability: Changes survive crashes (via Write-Ahead Log)
"""

import mongolite
import os
import tempfile


def example_basic_transaction():
    """Example 1: Basic transaction with commit"""
    print("\n" + "=" * 60)
    print("Example 1: Basic Transaction")
    print("=" * 60)

    with tempfile.TemporaryDirectory() as tmpdir:
        db_path = os.path.join(tmpdir, "example.mlite")
        db = mongolite.MongoLite(db_path)

        # Start a transaction
        tx_id = db.begin_transaction()
        print(f"✓ Started transaction: {tx_id}")

        # Insert documents within the transaction
        result1 = db.insert_one_tx("users", {"name": "Alice", "age": 30}, tx_id)
        print(f"✓ Inserted Alice: ID {result1['inserted_id']}")

        result2 = db.insert_one_tx("users", {"name": "Bob", "age": 25}, tx_id)
        print(f"✓ Inserted Bob: ID {result2['inserted_id']}")

        # Commit the transaction
        db.commit_transaction(tx_id)
        print(f"✓ Committed transaction: {tx_id}")

        # Verify data was persisted
        users = db.collection("users")
        count = users.count_documents({})
        print(f"✓ Total users after commit: {count}")


def example_transaction_rollback():
    """Example 2: Transaction rollback on error"""
    print("\n" + "=" * 60)
    print("Example 2: Transaction Rollback")
    print("=" * 60)

    with tempfile.TemporaryDirectory() as tmpdir:
        db_path = os.path.join(tmpdir, "example.mlite")
        db = mongolite.MongoLite(db_path)

        # Insert initial data outside transaction
        users = db.collection("users")
        users.insert_one({"name": "Alice", "age": 30})
        print("✓ Inserted Alice outside transaction")

        # Start a transaction
        tx_id = db.begin_transaction()
        print(f"✓ Started transaction: {tx_id}")

        # Insert documents in transaction
        db.insert_one_tx("users", {"name": "Bob", "age": 25}, tx_id)
        db.insert_one_tx("users", {"name": "Charlie", "age": 35}, tx_id)
        print("✓ Inserted Bob and Charlie in transaction")

        # Simulate an error condition and rollback
        error_occurred = True
        if error_occurred:
            db.rollback_transaction(tx_id)
            print(f"✓ Rolled back transaction {tx_id} due to error")
        else:
            db.commit_transaction(tx_id)

        # Verify only Alice exists (Bob and Charlie were rolled back)
        count = users.count_documents({})
        print(f"✓ Total users after rollback: {count} (only Alice)")


def example_multiple_transactions():
    """Example 3: Multi-operation transaction (Insert/Update/Delete)"""
    print("\n" + "=" * 60)
    print("Example 3: Multi-Operation Transaction")
    print("=" * 60)

    with tempfile.TemporaryDirectory() as tmpdir:
        db_path = os.path.join(tmpdir, "example.mlite")
        db = mongolite.MongoLite(db_path)

        # Setup initial data
        users = db.collection("users")
        users.insert_one({"name": "Alice", "age": 30})
        users.insert_one({"name": "Bob", "age": 25})
        users.insert_one({"name": "Charlie", "age": 35})
        print("✓ Setup: 3 users (Alice, Bob, Charlie)")

        # Begin transaction with multiple operations
        tx_id = db.begin_transaction()
        print(f"✓ Started transaction: {tx_id}")

        # INSERT new user
        db.insert_one_tx("users", {"name": "Diana", "age": 28}, tx_id)
        print("  - INSERT: Diana")

        # UPDATE existing user
        db.update_one_tx("users", {"name": "Alice"}, {"name": "Alice", "age": 31}, tx_id)
        print("  - UPDATE: Alice's age 30 → 31")

        # DELETE existing user
        db.delete_one_tx("users", {"name": "Bob"}, tx_id)
        print("  - DELETE: Bob")

        # Commit all changes atomically
        db.commit_transaction(tx_id)
        print(f"✓ Committed transaction: {tx_id}")

        # Verify final state
        final_count = users.count_documents({})
        print(f"✓ Final user count: {final_count} (Alice, Charlie, Diana)")


def example_error_handling():
    """Example 4: Multi-collection transaction"""
    print("\n" + "=" * 60)
    print("Example 4: Multi-Collection Transaction")
    print("=" * 60)

    with tempfile.TemporaryDirectory() as tmpdir:
        db_path = os.path.join(tmpdir, "example.mlite")
        db = mongolite.MongoLite(db_path)

        # Begin transaction affecting multiple collections
        tx_id = db.begin_transaction()
        print(f"✓ Started transaction: {tx_id}")

        # Insert into multiple collections atomically
        db.insert_one_tx("users", {"name": "Alice", "role": "admin"}, tx_id)
        db.insert_one_tx("logs", {"action": "user_created", "user": "Alice"}, tx_id)
        db.insert_one_tx("settings", {"key": "max_users", "value": 100}, tx_id)
        print("✓ Inserted into 3 collections (users, logs, settings)")

        # Commit all collections atomically
        db.commit_transaction(tx_id)
        print(f"✓ Committed transaction: {tx_id}")

        # Verify data in all collections
        users_count = db.collection("users").count_documents({})
        logs_count = db.collection("logs").count_documents({})
        settings_count = db.collection("settings").count_documents({})
        print(f"✓ Final state: users={users_count}, logs={logs_count}, settings={settings_count}")


def example_transaction_lifecycle():
    """Example 5: Proper error handling with transactions"""
    print("\n" + "=" * 60)
    print("Example 5: Error Handling Pattern")
    print("=" * 60)

    with tempfile.TemporaryDirectory() as tmpdir:
        db_path = os.path.join(tmpdir, "example.mlite")
        db = mongolite.MongoLite(db_path)

        # Recommended error handling pattern
        tx_id = db.begin_transaction()
        print(f"✓ Started transaction: {tx_id}")

        try:
            # Perform operations
            db.insert_one_tx("users", {"name": "Alice", "age": 30}, tx_id)
            db.insert_one_tx("users", {"name": "Bob", "age": 25}, tx_id)
            print("✓ Performed operations")

            # Business logic validation (example)
            users = db.collection("users")
            # Validation would happen here...

            # Commit if all operations succeeded
            db.commit_transaction(tx_id)
            print(f"✓ Transaction committed successfully")

        except Exception as e:
            # Rollback on any error
            db.rollback_transaction(tx_id)
            print(f"✗ Transaction rolled back due to error: {e}")


def main():
    """Run all examples"""
    print("=" * 60)
    print("MongoLite ACD Transactions - Python Examples")
    print("=" * 60)
    print("\nACD Features:")
    print("✓ Atomicity: All operations succeed or fail together")
    print("✓ Consistency: Data integrity is maintained")
    print("✓ Durability: Changes survive crashes (via WAL)")

    example_basic_transaction()
    example_transaction_rollback()
    example_multiple_transactions()
    example_error_handling()
    example_transaction_lifecycle()

    print("\n" + "=" * 60)
    print("All examples completed successfully!")
    print("=" * 60)
    print("\nKey Takeaways:")
    print("✓ Use db.begin_transaction() to start")
    print("✓ Use db.insert_one_tx(collection, doc, tx_id) for inserts")
    print("✓ Use db.update_one_tx(collection, query, new_doc, tx_id) for updates")
    print("✓ Use db.delete_one_tx(collection, query, tx_id) for deletes")
    print("✓ Use db.commit_transaction(tx_id) to commit")
    print("✓ Use db.rollback_transaction(tx_id) to rollback")
    print("✓ Always use try/except for proper error handling")
    print("=" * 60)


if __name__ == "__main__":
    main()
