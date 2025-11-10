#!/usr/bin/env python3
"""
MongoLite Test Suite - Test First Development

Run with: python test_ironbase.py
"""

import os
import sys
from pathlib import Path

# Test database path
TEST_DB = "test_ironbase.mlite"


def cleanup_test_db():
    """Remove test database if exists"""
    if Path(TEST_DB).exists():
        os.remove(TEST_DB)


def test_find_all_documents():
    """Test find() without query returns all documents"""
    print("TEST: find() all documents...", end=" ")

    cleanup_test_db()

    try:
        from ironbase import MongoLite

        db = MongoLite(TEST_DB)
        collection = db.collection("users")

        # Insert test data
        collection.insert_one({"name": "Alice", "age": 25})
        collection.insert_one({"name": "Bob", "age": 30})
        collection.insert_one({"name": "Charlie", "age": 35})

        # Find all
        results = collection.find({})

        assert len(results) == 3, f"Expected 3 docs, got {len(results)}"
        # Check all names are present (order not guaranteed with HashMap)
        names = {doc["name"] for doc in results}
        assert names == {"Alice", "Bob", "Charlie"}, f"Expected all names, got {names}"

        db.close()
        print("✅ PASS")
        return True

    except Exception as e:
        print(f"❌ FAIL: {e}")
        return False
    finally:
        cleanup_test_db()


def test_find_with_equality_query():
    """Test find() with simple equality query"""
    print("TEST: find() with equality...", end=" ")

    cleanup_test_db()

    try:
        from ironbase import MongoLite

        db = MongoLite(TEST_DB)
        collection = db.collection("users")

        # Insert test data
        collection.insert_one({"name": "Alice", "age": 25, "city": "NYC"})
        collection.insert_one({"name": "Bob", "age": 30, "city": "LA"})
        collection.insert_one({"name": "Charlie", "age": 25, "city": "NYC"})

        # Find by age
        results = collection.find({"age": 25})

        assert len(results) == 2, f"Expected 2 docs, got {len(results)}"
        assert all(doc["age"] == 25 for doc in results)

        # Find by city
        results = collection.find({"city": "LA"})

        assert len(results) == 1
        assert results[0]["name"] == "Bob"

        db.close()
        print("✅ PASS")
        return True

    except Exception as e:
        print(f"❌ FAIL: {e}")
        return False
    finally:
        cleanup_test_db()


def test_find_with_comparison_operators():
    """Test find() with $gt, $lt, $gte, $lte"""
    print("TEST: find() with comparison operators...", end=" ")

    cleanup_test_db()

    try:
        from ironbase import MongoLite

        db = MongoLite(TEST_DB)
        collection = db.collection("users")

        # Insert test data
        for i in range(10, 41, 10):  # 10, 20, 30, 40
            collection.insert_one({"name": f"User{i}", "age": i})

        # Test $gt
        results = collection.find({"age": {"$gt": 20}})
        assert len(results) == 2, f"$gt: Expected 2, got {len(results)}"
        assert all(doc["age"] > 20 for doc in results)

        # Test $gte
        results = collection.find({"age": {"$gte": 20}})
        assert len(results) == 3, f"$gte: Expected 3, got {len(results)}"

        # Test $lt
        results = collection.find({"age": {"$lt": 30}})
        assert len(results) == 2, f"$lt: Expected 2, got {len(results)}"

        # Test $lte
        results = collection.find({"age": {"$lte": 30}})
        assert len(results) == 3, f"$lte: Expected 3, got {len(results)}"

        db.close()
        print("✅ PASS")
        return True

    except Exception as e:
        print(f"❌ FAIL: {e}")
        return False
    finally:
        cleanup_test_db()


def test_find_one():
    """Test find_one() returns single document"""
    print("TEST: find_one()...", end=" ")

    cleanup_test_db()

    try:
        from ironbase import MongoLite

        db = MongoLite(TEST_DB)
        collection = db.collection("users")

        # Insert test data
        collection.insert_one({"name": "Alice", "age": 25})
        collection.insert_one({"name": "Bob", "age": 30})

        # Find one by name
        result = collection.find_one({"name": "Bob"})

        assert result is not None
        assert result["name"] == "Bob"
        assert result["age"] == 30

        # Find one that doesn't exist
        result = collection.find_one({"name": "Charlie"})

        assert result is None

        db.close()
        print("✅ PASS")
        return True

    except Exception as e:
        print(f"❌ FAIL: {e}")
        return False
    finally:
        cleanup_test_db()


def test_find_with_in_operator():
    """Test find() with $in operator"""
    print("TEST: find() with $in...", end=" ")

    cleanup_test_db()

    try:
        from ironbase import MongoLite

        db = MongoLite(TEST_DB)
        collection = db.collection("users")

        # Insert test data
        collection.insert_one({"name": "Alice", "role": "admin"})
        collection.insert_one({"name": "Bob", "role": "user"})
        collection.insert_one({"name": "Charlie", "role": "moderator"})
        collection.insert_one({"name": "Dave", "role": "user"})

        # Find with $in
        results = collection.find({"role": {"$in": ["admin", "moderator"]}})

        assert len(results) == 2
        roles = [doc["role"] for doc in results]
        assert "admin" in roles
        assert "moderator" in roles
        assert "user" not in roles

        db.close()
        print("✅ PASS")
        return True

    except Exception as e:
        print(f"❌ FAIL: {e}")
        return False
    finally:
        cleanup_test_db()


def test_find_empty_collection():
    """Test find() on empty collection"""
    print("TEST: find() on empty collection...", end=" ")

    cleanup_test_db()

    try:
        from ironbase import MongoLite

        db = MongoLite(TEST_DB)
        collection = db.collection("empty")

        results = collection.find({})

        assert len(results) == 0
        assert isinstance(results, list)

        db.close()
        print("✅ PASS")
        return True

    except Exception as e:
        print(f"❌ FAIL: {e}")
        return False
    finally:
        cleanup_test_db()


def run_all_tests():
    """Run all test functions"""
    print("=" * 60)
    print("MongoLite Test Suite - Find Operations")
    print("=" * 60)
    print()

    tests = [
        test_find_all_documents,
        test_find_with_equality_query,
        test_find_with_comparison_operators,
        test_find_one,
        test_find_with_in_operator,
        test_find_empty_collection,
    ]

    passed = 0
    failed = 0

    for test in tests:
        if test():
            passed += 1
        else:
            failed += 1

    print()
    print("=" * 60)
    print(f"Results: {passed} passed, {failed} failed out of {len(tests)} tests")
    print("=" * 60)

    return failed == 0


if __name__ == "__main__":
    success = run_all_tests()
    sys.exit(0 if success else 1)
