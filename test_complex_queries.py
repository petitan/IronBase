#!/usr/bin/env python3
"""
MongoLite Complex Query Operations Test Suite

Test-first approach: Writing tests before implementation.
Tests for $and, $or, $not, $ne operators.
"""
import ironbase
import os

TEST_DB = "test_complex_queries.db"

def cleanup_test_db():
    """Remove test database if exists"""
    if os.path.exists(TEST_DB):
        os.remove(TEST_DB)

def test_ne_operator():
    """Test $ne (not equal) operator"""
    cleanup_test_db()
    db = ironbase.MongoLite(TEST_DB)
    collection = db.collection("users")

    # Insert test data
    collection.insert_many([
        {"name": "Alice", "age": 25, "city": "NYC"},
        {"name": "Bob", "age": 30, "city": "LA"},
        {"name": "Charlie", "age": 25, "city": "Chicago"},
        {"name": "Dave", "age": 35, "city": "NYC"},
    ])

    # Find all where age != 25
    results = collection.find({"age": {"$ne": 25}})

    assert len(results) == 2, f"Expected 2 docs with age != 25, got {len(results)}"
    names = {doc["name"] for doc in results}
    assert names == {"Bob", "Dave"}, f"Expected Bob and Dave, got {names}"

    print("✅ PASS")

def test_and_operator():
    """Test $and operator"""
    cleanup_test_db()
    db = ironbase.MongoLite(TEST_DB)
    collection = db.collection("products")

    # Insert test data
    collection.insert_many([
        {"name": "Widget", "category": "Tools", "price": 10},
        {"name": "Gadget", "category": "Tools", "price": 25},
        {"name": "Gizmo", "category": "Electronics", "price": 20},
        {"name": "Doohickey", "category": "Tools", "price": 35},
    ])

    # Find Tools with price > 20
    results = collection.find({
        "$and": [
            {"category": "Tools"},
            {"price": {"$gt": 20}}
        ]
    })

    assert len(results) == 2, f"Expected 2 docs, got {len(results)}"
    names = {doc["name"] for doc in results}
    assert names == {"Gadget", "Doohickey"}, f"Expected Gadget and Doohickey, got {names}"

    print("✅ PASS")

def test_or_operator():
    """Test $or operator"""
    cleanup_test_db()
    db = ironbase.MongoLite(TEST_DB)
    collection = db.collection("users")

    # Insert test data
    collection.insert_many([
        {"name": "Alice", "age": 25, "city": "NYC"},
        {"name": "Bob", "age": 30, "city": "LA"},
        {"name": "Charlie", "age": 25, "city": "Chicago"},
        {"name": "Dave", "age": 35, "city": "NYC"},
    ])

    # Find age 25 OR city NYC
    results = collection.find({
        "$or": [
            {"age": 25},
            {"city": "NYC"}
        ]
    })

    assert len(results) == 3, f"Expected 3 docs, got {len(results)}"
    names = {doc["name"] for doc in results}
    # Alice: age 25 and NYC (matches both)
    # Charlie: age 25
    # Dave: NYC
    assert names == {"Alice", "Charlie", "Dave"}, f"Expected Alice, Charlie, Dave, got {names}"

    print("✅ PASS")

def test_not_operator():
    """Test $not operator"""
    cleanup_test_db()
    db = ironbase.MongoLite(TEST_DB)
    collection = db.collection("users")

    # Insert test data
    collection.insert_many([
        {"name": "Alice", "age": 25},
        {"name": "Bob", "age": 30},
        {"name": "Charlie", "age": 35},
        {"name": "Dave", "age": 40},
    ])

    # Find NOT (age > 30)
    results = collection.find({
        "age": {"$not": {"$gt": 30}}
    })

    assert len(results) == 2, f"Expected 2 docs with age <= 30, got {len(results)}"
    names = {doc["name"] for doc in results}
    assert names == {"Alice", "Bob"}, f"Expected Alice and Bob, got {names}"

    print("✅ PASS")

def test_complex_and_or():
    """Test combination of $and and $or"""
    cleanup_test_db()
    db = ironbase.MongoLite(TEST_DB)
    collection = db.collection("products")

    # Insert test data
    collection.insert_many([
        {"name": "Widget", "category": "Tools", "price": 10, "inStock": True},
        {"name": "Gadget", "category": "Tools", "price": 25, "inStock": False},
        {"name": "Gizmo", "category": "Electronics", "price": 20, "inStock": True},
        {"name": "Doohickey", "category": "Tools", "price": 35, "inStock": True},
        {"name": "Thingamajig", "category": "Electronics", "price": 15, "inStock": False},
    ])

    # Find (Tools OR Electronics) AND inStock AND price < 30
    results = collection.find({
        "$and": [
            {
                "$or": [
                    {"category": "Tools"},
                    {"category": "Electronics"}
                ]
            },
            {"inStock": True},
            {"price": {"$lt": 30}}
        ]
    })

    assert len(results) == 2, f"Expected 2 docs, got {len(results)}"
    names = {doc["name"] for doc in results}
    # Widget: Tools, inStock, price 10 < 30 ✓
    # Gizmo: Electronics, inStock, price 20 < 30 ✓
    # Gadget: Tools but not inStock ✗
    # Doohickey: Tools, inStock but price 35 >= 30 ✗
    assert names == {"Widget", "Gizmo"}, f"Expected Widget and Gizmo, got {names}"

    print("✅ PASS")

def test_ne_with_missing_field():
    """Test $ne with missing field"""
    cleanup_test_db()
    db = ironbase.MongoLite(TEST_DB)
    collection = db.collection("users")

    # Insert test data
    collection.insert_many([
        {"name": "Alice", "age": 25},
        {"name": "Bob", "age": 30},
        {"name": "Charlie"},  # No age field
    ])

    # Find age != 25 (should include Charlie since missing field != 25)
    results = collection.find({"age": {"$ne": 25}})

    assert len(results) == 2, f"Expected 2 docs, got {len(results)}"
    names = {doc["name"] for doc in results}
    assert names == {"Bob", "Charlie"}, f"Expected Bob and Charlie, got {names}"

    print("✅ PASS")

def run_tests():
    """Run all complex query tests"""
    print("=" * 60)
    print("MongoLite Test Suite - Complex Query Operations")
    print("=" * 60)
    print()

    tests = [
        ("$ne operator", test_ne_operator),
        ("$and operator", test_and_operator),
        ("$or operator", test_or_operator),
        ("$not operator", test_not_operator),
        ("complex $and + $or", test_complex_and_or),
        ("$ne with missing field", test_ne_with_missing_field),
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
