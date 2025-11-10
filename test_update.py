#!/usr/bin/env python3
"""
MongoLite Update Operations Test Suite

Test-first approach: Writing tests before implementation.
"""
import ironbase
import os

TEST_DB = "test_update.db"

def cleanup_test_db():
    """Remove test database if exists"""
    if os.path.exists(TEST_DB):
        os.remove(TEST_DB)

def test_update_one_set():
    """Test update_one() with $set operator"""
    cleanup_test_db()
    db = ironbase.MongoLite(TEST_DB)
    collection = db.collection("users")

    # Insert test data
    collection.insert_one({"name": "Alice", "age": 25, "city": "NYC"})
    collection.insert_one({"name": "Bob", "age": 30, "city": "LA"})

    # Update Alice's age and city
    result = collection.update_one(
        {"name": "Alice"},
        {"$set": {"age": 26, "city": "SF"}}
    )

    assert result["acknowledged"] == True
    assert result["matched_count"] == 1
    assert result["modified_count"] == 1

    # Verify update
    alice = collection.find_one({"name": "Alice"})
    assert alice["age"] == 26
    assert alice["city"] == "SF"
    assert alice["name"] == "Alice"  # Unchanged

    print("✅ PASS")

def test_update_one_inc():
    """Test update_one() with $inc operator"""
    cleanup_test_db()
    db = ironbase.MongoLite(TEST_DB)
    collection = db.collection("counters")

    collection.insert_one({"name": "page_views", "count": 100})

    # Increment counter
    result = collection.update_one(
        {"name": "page_views"},
        {"$inc": {"count": 5}}
    )

    assert result["acknowledged"] == True
    assert result["matched_count"] == 1
    assert result["modified_count"] == 1

    # Verify
    doc = collection.find_one({"name": "page_views"})
    assert doc["count"] == 105

    # Negative increment
    collection.update_one(
        {"name": "page_views"},
        {"$inc": {"count": -10}}
    )
    doc = collection.find_one({"name": "page_views"})
    assert doc["count"] == 95

    print("✅ PASS")

def test_update_one_unset():
    """Test update_one() with $unset operator"""
    cleanup_test_db()
    db = ironbase.MongoLite(TEST_DB)
    collection = db.collection("users")

    collection.insert_one({"name": "Charlie", "age": 35, "email": "charlie@example.com"})

    # Remove email field
    result = collection.update_one(
        {"name": "Charlie"},
        {"$unset": {"email": ""}}
    )

    assert result["acknowledged"] == True
    assert result["matched_count"] == 1
    assert result["modified_count"] == 1

    # Verify email is removed
    doc = collection.find_one({"name": "Charlie"})
    assert "email" not in doc
    assert doc["name"] == "Charlie"
    assert doc["age"] == 35

    print("✅ PASS")

def test_update_one_no_match():
    """Test update_one() when no document matches"""
    cleanup_test_db()
    db = ironbase.MongoLite(TEST_DB)
    collection = db.collection("users")

    collection.insert_one({"name": "Alice", "age": 25})

    result = collection.update_one(
        {"name": "NonExistent"},
        {"$set": {"age": 30}}
    )

    assert result["acknowledged"] == True
    assert result["matched_count"] == 0
    assert result["modified_count"] == 0

    print("✅ PASS")

def test_update_one_multiple_operators():
    """Test update_one() with multiple operators"""
    cleanup_test_db()
    db = ironbase.MongoLite(TEST_DB)
    collection = db.collection("products")

    collection.insert_one({
        "name": "Widget",
        "price": 19.99,
        "stock": 100,
        "discontinued": False
    })

    # Apply multiple updates
    result = collection.update_one(
        {"name": "Widget"},
        {
            "$set": {"price": 24.99},
            "$inc": {"stock": -10},
            "$unset": {"discontinued": ""}
        }
    )

    assert result["acknowledged"] == True
    assert result["matched_count"] == 1
    assert result["modified_count"] == 1

    # Verify all changes
    doc = collection.find_one({"name": "Widget"})
    assert doc["price"] == 24.99
    assert doc["stock"] == 90
    assert "discontinued" not in doc

    print("✅ PASS")

def test_update_many_set():
    """Test update_many() updates all matching documents"""
    cleanup_test_db()
    db = ironbase.MongoLite(TEST_DB)
    collection = db.collection("employees")

    # Insert test data
    collection.insert_many([
        {"name": "Alice", "department": "Engineering", "salary": 80000},
        {"name": "Bob", "department": "Engineering", "salary": 85000},
        {"name": "Charlie", "department": "Sales", "salary": 70000},
    ])

    # Give Engineering department a raise
    result = collection.update_many(
        {"department": "Engineering"},
        {"$inc": {"salary": 5000}}
    )

    assert result["acknowledged"] == True
    assert result["matched_count"] == 2
    assert result["modified_count"] == 2

    # Verify updates
    engineers = collection.find({"department": "Engineering"})
    assert len(engineers) == 2
    assert all(e["salary"] >= 85000 for e in engineers)

    print("✅ PASS")

def run_tests():
    """Run all update tests"""
    print("=" * 60)
    print("MongoLite Test Suite - Update Operations")
    print("=" * 60)
    print()

    tests = [
        ("update_one() with $set", test_update_one_set),
        ("update_one() with $inc", test_update_one_inc),
        ("update_one() with $unset", test_update_one_unset),
        ("update_one() no match", test_update_one_no_match),
        ("update_one() multiple operators", test_update_one_multiple_operators),
        ("update_many() with $set", test_update_many_set),
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
