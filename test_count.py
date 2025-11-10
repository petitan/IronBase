#!/usr/bin/env python3
"""
MongoLite Count Operations Test Suite

Test-first approach: Writing tests before implementation.
"""
import ironbase
import os

TEST_DB = "test_count.db"

def cleanup_test_db():
    """Remove test database if exists"""
    if os.path.exists(TEST_DB):
        os.remove(TEST_DB)

def test_count_all():
    """Test count_documents() without filter returns total count"""
    cleanup_test_db()
    db = ironbase.MongoLite(TEST_DB)
    collection = db.collection("users")

    # Insert test data
    collection.insert_many([
        {"name": "Alice", "age": 25},
        {"name": "Bob", "age": 30},
        {"name": "Charlie", "age": 35},
    ])

    # Count all documents
    count = collection.count_documents({})

    assert count == 3, f"Expected 3, got {count}"

    print("✅ PASS")

def test_count_with_filter():
    """Test count_documents() with filter"""
    cleanup_test_db()
    db = ironbase.MongoLite(TEST_DB)
    collection = db.collection("products")

    # Insert test data
    collection.insert_many([
        {"name": "Widget", "price": 10, "category": "Tools"},
        {"name": "Gadget", "price": 15, "category": "Tools"},
        {"name": "Gizmo", "price": 20, "category": "Electronics"},
        {"name": "Doohickey", "price": 5, "category": "Tools"},
    ])

    # Count with filter
    tools_count = collection.count_documents({"category": "Tools"})
    electronics_count = collection.count_documents({"category": "Electronics"})

    assert tools_count == 3, f"Expected 3 tools, got {tools_count}"
    assert electronics_count == 1, f"Expected 1 electronics, got {electronics_count}"

    print("✅ PASS")

def test_count_with_comparison():
    """Test count_documents() with comparison operators"""
    cleanup_test_db()
    db = ironbase.MongoLite(TEST_DB)
    collection = db.collection("users")

    # Insert test data
    for i in range(10, 51, 10):  # 10, 20, 30, 40, 50
        collection.insert_one({"name": f"User{i}", "age": i})

    # Count with $gt
    count_over_25 = collection.count_documents({"age": {"$gt": 25}})
    assert count_over_25 == 3, f"Expected 3, got {count_over_25}"

    # Count with $lte
    count_25_or_less = collection.count_documents({"age": {"$lte": 25}})
    assert count_25_or_less == 2, f"Expected 2, got {count_25_or_less}"

    print("✅ PASS")

def test_count_empty_collection():
    """Test count_documents() on empty collection"""
    cleanup_test_db()
    db = ironbase.MongoLite(TEST_DB)
    collection = db.collection("empty")

    count = collection.count_documents({})

    assert count == 0, f"Expected 0, got {count}"

    print("✅ PASS")

def test_count_no_match():
    """Test count_documents() when no documents match"""
    cleanup_test_db()
    db = ironbase.MongoLite(TEST_DB)
    collection = db.collection("users")

    collection.insert_many([
        {"name": "Alice", "age": 25},
        {"name": "Bob", "age": 30},
    ])

    # Count with no matches
    count = collection.count_documents({"age": {"$gt": 100}})

    assert count == 0, f"Expected 0, got {count}"

    print("✅ PASS")

def test_count_after_delete():
    """Test count_documents() after deleting documents"""
    cleanup_test_db()
    db = ironbase.MongoLite(TEST_DB)
    collection = db.collection("users")

    # Insert and count
    collection.insert_many([
        {"name": "Alice", "age": 25},
        {"name": "Bob", "age": 30},
        {"name": "Charlie", "age": 35},
    ])

    count_before = collection.count_documents({})
    assert count_before == 3

    # Delete and count again
    collection.delete_one({"name": "Bob"})
    count_after = collection.count_documents({})

    assert count_after == 2, f"Expected 2 after delete, got {count_after}"

    print("✅ PASS")

def run_tests():
    """Run all count tests"""
    print("=" * 60)
    print("MongoLite Test Suite - Count Operations")
    print("=" * 60)
    print()

    tests = [
        ("count all documents", test_count_all),
        ("count with filter", test_count_with_filter),
        ("count with comparison operators", test_count_with_comparison),
        ("count empty collection", test_count_empty_collection),
        ("count no match", test_count_no_match),
        ("count after delete", test_count_after_delete),
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
