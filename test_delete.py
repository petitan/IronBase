#!/usr/bin/env python3
"""
MongoLite Delete Operations Test Suite

Test-first approach: Writing tests before implementation.
"""
import ironbase
import os

TEST_DB = "test_delete.db"

def cleanup_test_db():
    """Remove test database if exists"""
    if os.path.exists(TEST_DB):
        os.remove(TEST_DB)

def test_delete_one():
    """Test delete_one() deletes single matching document"""
    cleanup_test_db()
    db = ironbase.MongoLite(TEST_DB)
    collection = db.collection("users")

    # Insert test data
    collection.insert_many([
        {"name": "Alice", "age": 25},
        {"name": "Bob", "age": 30},
        {"name": "Alice", "age": 35},  # Duplicate name
    ])

    # Delete first Alice
    result = collection.delete_one({"name": "Alice"})

    assert result["acknowledged"] == True
    assert result["deleted_count"] == 1

    # Verify: should have 2 documents left
    remaining = collection.find({})
    assert len(remaining) == 2

    # One Alice should remain
    alices = collection.find({"name": "Alice"})
    assert len(alices) == 1

    print("✅ PASS")

def test_delete_many():
    """Test delete_many() deletes all matching documents"""
    cleanup_test_db()
    db = ironbase.MongoLite(TEST_DB)
    collection = db.collection("products")

    # Insert test data
    collection.insert_many([
        {"name": "Widget", "category": "Tools", "price": 10},
        {"name": "Gadget", "category": "Tools", "price": 15},
        {"name": "Gizmo", "category": "Electronics", "price": 20},
        {"name": "Doohickey", "category": "Tools", "price": 5},
    ])

    # Delete all Tools
    result = collection.delete_many({"category": "Tools"})

    assert result["acknowledged"] == True
    assert result["deleted_count"] == 3

    # Verify: only Electronics remain
    remaining = collection.find({})
    assert len(remaining) == 1
    assert remaining[0]["category"] == "Electronics"

    print("✅ PASS")

def test_delete_one_no_match():
    """Test delete_one() when no document matches"""
    cleanup_test_db()
    db = ironbase.MongoLite(TEST_DB)
    collection = db.collection("users")

    collection.insert_one({"name": "Alice", "age": 25})

    # Try to delete non-existent
    result = collection.delete_one({"name": "NonExistent"})

    assert result["acknowledged"] == True
    assert result["deleted_count"] == 0

    # Verify: Alice still exists
    remaining = collection.find({})
    assert len(remaining) == 1

    print("✅ PASS")

def test_delete_many_no_match():
    """Test delete_many() when no documents match"""
    cleanup_test_db()
    db = ironbase.MongoLite(TEST_DB)
    collection = db.collection("users")

    collection.insert_many([
        {"name": "Alice", "age": 25},
        {"name": "Bob", "age": 30},
    ])

    # Try to delete non-existent
    result = collection.delete_many({"age": {"$gt": 50}})

    assert result["acknowledged"] == True
    assert result["deleted_count"] == 0

    # Verify: all remain
    remaining = collection.find({})
    assert len(remaining) == 2

    print("✅ PASS")

def test_delete_then_find():
    """Test that deleted documents don't appear in find() results"""
    cleanup_test_db()
    db = ironbase.MongoLite(TEST_DB)
    collection = db.collection("users")

    collection.insert_many([
        {"name": "Alice", "age": 25, "active": True},
        {"name": "Bob", "age": 30, "active": False},
        {"name": "Charlie", "age": 35, "active": True},
    ])

    # Delete inactive users
    collection.delete_many({"active": False})

    # Verify Bob is gone
    all_users = collection.find({})
    assert len(all_users) == 2
    names = {doc["name"] for doc in all_users}
    assert names == {"Alice", "Charlie"}
    assert "Bob" not in names

    print("✅ PASS")

def test_delete_then_update():
    """Test that deleted documents can't be updated"""
    cleanup_test_db()
    db = ironbase.MongoLite(TEST_DB)
    collection = db.collection("users")

    collection.insert_one({"name": "Alice", "age": 25})

    # Delete Alice
    collection.delete_one({"name": "Alice"})

    # Try to update deleted document
    result = collection.update_one({"name": "Alice"}, {"$set": {"age": 30}})

    assert result["matched_count"] == 0
    assert result["modified_count"] == 0

    # Verify no documents
    all_docs = collection.find({})
    assert len(all_docs) == 0

    print("✅ PASS")

def run_tests():
    """Run all delete tests"""
    print("=" * 60)
    print("MongoLite Test Suite - Delete Operations")
    print("=" * 60)
    print()

    tests = [
        ("delete_one() single match", test_delete_one),
        ("delete_many() multiple matches", test_delete_many),
        ("delete_one() no match", test_delete_one_no_match),
        ("delete_many() no match", test_delete_many_no_match),
        ("delete then find", test_delete_then_find),
        ("delete then update", test_delete_then_update),
    ]

    passed = 0
    failed = 0

    for name, test_func in tests:
        print(f"TEST: {name}... ", end="", flush=True)
        try:
            test_func()
            passed += 1
        except AssertionError as e:
            print(f"❌ FAIL: {e}")
            failed += 1
        except Exception as e:
            print(f"❌ FAIL: {e}")
            failed += 1

    print()
    print("=" * 60)
    print(f"Results: {passed} passed, {failed} failed out of {passed + failed} tests")
    print("=" * 60)

    # Cleanup
    cleanup_test_db()

    return failed == 0

if __name__ == "__main__":
    import sys
    success = run_tests()
    sys.exit(0 if success else 1)
