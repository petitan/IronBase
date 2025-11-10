#!/usr/bin/env python3
"""
MongoLite Distinct Operations Test Suite

Test-first approach: Writing tests before implementation.
"""
import ironbase
import os

TEST_DB = "test_distinct.db"

def cleanup_test_db():
    """Remove test database if exists"""
    if os.path.exists(TEST_DB):
        os.remove(TEST_DB)

def test_distinct_basic():
    """Test distinct() returns unique values"""
    cleanup_test_db()
    db = ironbase.MongoLite(TEST_DB)
    collection = db.collection("users")

    # Insert test data with duplicate values
    collection.insert_many([
        {"name": "Alice", "age": 25, "city": "NYC"},
        {"name": "Bob", "age": 30, "city": "LA"},
        {"name": "Charlie", "age": 25, "city": "NYC"},
        {"name": "Dave", "age": 30, "city": "Chicago"},
        {"name": "Eve", "age": 25, "city": "LA"},
    ])

    # Get distinct ages
    ages = collection.distinct("age")

    assert len(ages) == 2, f"Expected 2 unique ages, got {len(ages)}"
    assert set(ages) == {25, 30}, f"Expected {{25, 30}}, got {set(ages)}"

    print("✅ PASS")

def test_distinct_strings():
    """Test distinct() with string values"""
    cleanup_test_db()
    db = ironbase.MongoLite(TEST_DB)
    collection = db.collection("users")

    # Insert test data
    collection.insert_many([
        {"name": "Alice", "city": "NYC"},
        {"name": "Bob", "city": "LA"},
        {"name": "Charlie", "city": "NYC"},
        {"name": "Dave", "city": "Chicago"},
        {"name": "Eve", "city": "LA"},
    ])

    # Get distinct cities
    cities = collection.distinct("city")

    assert len(cities) == 3, f"Expected 3 unique cities, got {len(cities)}"
    assert set(cities) == {"NYC", "LA", "Chicago"}, f"Expected cities, got {set(cities)}"

    print("✅ PASS")

def test_distinct_with_filter():
    """Test distinct() with query filter"""
    cleanup_test_db()
    db = ironbase.MongoLite(TEST_DB)
    collection = db.collection("products")

    # Insert test data
    collection.insert_many([
        {"name": "Widget", "category": "Tools", "price": 10},
        {"name": "Gadget", "category": "Tools", "price": 15},
        {"name": "Gizmo", "category": "Electronics", "price": 20},
        {"name": "Doohickey", "category": "Tools", "price": 10},
        {"name": "Thingamajig", "category": "Electronics", "price": 15},
    ])

    # Get distinct prices for Tools category
    tool_prices = collection.distinct("price", {"category": "Tools"})

    assert len(tool_prices) == 2, f"Expected 2 unique prices, got {len(tool_prices)}"
    assert set(tool_prices) == {10, 15}, f"Expected {{10, 15}}, got {set(tool_prices)}"

    print("✅ PASS")

def test_distinct_empty_collection():
    """Test distinct() on empty collection"""
    cleanup_test_db()
    db = ironbase.MongoLite(TEST_DB)
    collection = db.collection("empty")

    values = collection.distinct("field")

    assert len(values) == 0, f"Expected empty list, got {len(values)} values"
    assert values == [], f"Expected [], got {values}"

    print("✅ PASS")

def test_distinct_missing_field():
    """Test distinct() with field that doesn't exist"""
    cleanup_test_db()
    db = ironbase.MongoLite(TEST_DB)
    collection = db.collection("users")

    collection.insert_many([
        {"name": "Alice", "age": 25},
        {"name": "Bob", "age": 30},
    ])

    # Get distinct on non-existent field
    values = collection.distinct("nonexistent")

    assert len(values) == 0, f"Expected empty list, got {len(values)} values"

    print("✅ PASS")

def test_distinct_after_delete():
    """Test distinct() after deleting documents"""
    cleanup_test_db()
    db = ironbase.MongoLite(TEST_DB)
    collection = db.collection("users")

    # Insert test data
    collection.insert_many([
        {"name": "Alice", "age": 25},
        {"name": "Bob", "age": 30},
        {"name": "Charlie", "age": 25},
        {"name": "Dave", "age": 35},
    ])

    # Delete all with age 25
    collection.delete_many({"age": 25})

    # Get distinct ages
    ages = collection.distinct("age")

    assert len(ages) == 2, f"Expected 2 ages after delete, got {len(ages)}"
    assert set(ages) == {30, 35}, f"Expected {{30, 35}}, got {set(ages)}"
    assert 25 not in ages, "Age 25 should not be present after delete"

    print("✅ PASS")

def run_tests():
    """Run all distinct tests"""
    print("=" * 60)
    print("MongoLite Test Suite - Distinct Operations")
    print("=" * 60)
    print()

    tests = [
        ("distinct basic", test_distinct_basic),
        ("distinct strings", test_distinct_strings),
        ("distinct with filter", test_distinct_with_filter),
        ("distinct empty collection", test_distinct_empty_collection),
        ("distinct missing field", test_distinct_missing_field),
        ("distinct after delete", test_distinct_after_delete),
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
