#!/usr/bin/env python3
"""
Python integration tests for array update operators
Tests: $push, $pull, $addToSet, $pop
"""

from ironbase import IronBase
import os

def cleanup():
    """Remove test database"""
    if os.path.exists("test_array_ops.mlite"):
        os.remove("test_array_ops.mlite")

def test_push_simple():
    """Test $push operator - simple value"""
    print("Test: $push simple value...")

    db = IronBase("test_array_ops.mlite")
    coll = db.collection("test")

    # Insert document with array
    coll.insert_one({"_id": 1, "tags": ["rust"]})

    # Push single element
    result = coll.update_one({"_id": 1}, {"$push": {"tags": "mongodb"}})
    assert result["matched_count"] == 1, f"Expected matched=1, got {result['matched_count']}"
    assert result["modified_count"] == 1, f"Expected modified=1, got {result['modified_count']}"

    # Verify
    docs = coll.find({"_id": 1})
    assert len(docs) == 1
    assert docs[0]["tags"] == ["rust", "mongodb"], f"Expected ['rust', 'mongodb'], got {docs[0]['tags']}"

    db.close()
    cleanup()
    print("  ✓ PASS")

def test_push_each():
    """Test $push with $each modifier"""
    print("Test: $push with $each...")

    db = IronBase("test_array_ops.mlite")
    coll = db.collection("test")

    # Insert document with array
    coll.insert_one({"_id": 1, "scores": [10, 20]})

    # Push multiple elements
    result = coll.update_one(
        {"_id": 1},
        {"$push": {"scores": {"$each": [30, 40, 50]}}}
    )
    assert result["matched_count"] == 1
    assert result["modified_count"] == 1

    # Verify
    docs = coll.find({"_id": 1})
    assert docs[0]["scores"] == [10, 20, 30, 40, 50]

    db.close()
    cleanup()
    print("  ✓ PASS")

def test_pull_simple():
    """Test $pull operator - simple equality"""
    print("Test: $pull simple equality...")

    db = IronBase("test_array_ops.mlite")
    coll = db.collection("test")

    # Insert document with array
    coll.insert_one({"_id": 1, "tags": ["rust", "python", "rust", "java"]})

    # Pull all "rust" elements
    result = coll.update_one({"_id": 1}, {"$pull": {"tags": "rust"}})
    assert result["matched_count"] == 1
    assert result["modified_count"] == 1

    # Verify
    docs = coll.find({"_id": 1})
    assert docs[0]["tags"] == ["python", "java"]

    db.close()
    cleanup()
    print("  ✓ PASS")

def test_pull_with_condition():
    """Test $pull with query condition"""
    print("Test: $pull with condition...")

    db = IronBase("test_array_ops.mlite")
    coll = db.collection("test")

    # Insert document with array of numbers
    coll.insert_one({"_id": 1, "scores": [5, 10, 15, 20, 25]})

    # Pull elements less than 15
    result = coll.update_one(
        {"_id": 1},
        {"$pull": {"scores": {"$lt": 15}}}
    )
    assert result["matched_count"] == 1
    assert result["modified_count"] == 1

    # Verify
    docs = coll.find({"_id": 1})
    assert docs[0]["scores"] == [15, 20, 25]

    db.close()
    cleanup()
    print("  ✓ PASS")

def test_addtoset_simple():
    """Test $addToSet operator - simple value"""
    print("Test: $addToSet simple...")

    db = IronBase("test_array_ops.mlite")
    coll = db.collection("test")

    # Insert document with array
    coll.insert_one({"_id": 1, "tags": ["rust", "python"]})

    # Add unique element
    result = coll.update_one(
        {"_id": 1},
        {"$addToSet": {"tags": "java"}}
    )
    assert result["matched_count"] == 1
    assert result["modified_count"] == 1

    # Verify
    docs = coll.find({"_id": 1})
    assert docs[0]["tags"] == ["rust", "python", "java"]

    db.close()
    cleanup()
    print("  ✓ PASS")

def test_addtoset_duplicate():
    """Test $addToSet with duplicate value (no-op)"""
    print("Test: $addToSet duplicate...")

    db = IronBase("test_array_ops.mlite")
    coll = db.collection("test")

    # Insert document with array
    coll.insert_one({"_id": 1, "tags": ["rust", "python"]})

    # Try to add duplicate element
    result = coll.update_one(
        {"_id": 1},
        {"$addToSet": {"tags": "rust"}}
    )
    assert result["matched_count"] == 1
    assert result["modified_count"] == 0, f"Expected modified=0 (no-op), got {result['modified_count']}"

    # Verify (array unchanged)
    docs = coll.find({"_id": 1})
    assert docs[0]["tags"] == ["rust", "python"]

    db.close()
    cleanup()
    print("  ✓ PASS")

def test_pop_first():
    """Test $pop operator - remove first element"""
    print("Test: $pop first element...")

    db = IronBase("test_array_ops.mlite")
    coll = db.collection("test")

    # Insert document with array
    coll.insert_one({"_id": 1, "items": [1, 2, 3, 4, 5]})

    # Pop first element
    result = coll.update_one({"_id": 1}, {"$pop": {"items": -1}})
    assert result["matched_count"] == 1
    assert result["modified_count"] == 1

    # Verify
    docs = coll.find({"_id": 1})
    assert docs[0]["items"] == [2, 3, 4, 5]

    db.close()
    cleanup()
    print("  ✓ PASS")

def test_pop_last():
    """Test $pop operator - remove last element"""
    print("Test: $pop last element...")

    db = IronBase("test_array_ops.mlite")
    coll = db.collection("test")

    # Insert document with array
    coll.insert_one({"_id": 1, "items": [1, 2, 3, 4, 5]})

    # Pop last element
    result = coll.update_one({"_id": 1}, {"$pop": {"items": 1}})
    assert result["matched_count"] == 1
    assert result["modified_count"] == 1

    # Verify
    docs = coll.find({"_id": 1})
    assert docs[0]["items"] == [1, 2, 3, 4]

    db.close()
    cleanup()
    print("  ✓ PASS")

def test_combined_operations():
    """Test multiple array operations in one update"""
    print("Test: Combined array operations...")

    db = IronBase("test_array_ops.mlite")
    coll = db.collection("test")

    # Insert document
    coll.insert_one({"_id": 1, "tags": ["a", "b"], "scores": [10, 20]})

    # Apply multiple array operations
    result = coll.update_one(
        {"_id": 1},
        {
            "$push": {"tags": "c"},
            "$addToSet": {"scores": 30}
        }
    )
    assert result["matched_count"] == 1
    assert result["modified_count"] == 1

    # Verify both operations applied
    docs = coll.find({"_id": 1})
    assert docs[0]["tags"] == ["a", "b", "c"]
    assert docs[0]["scores"] == [10, 20, 30]

    db.close()
    cleanup()
    print("  ✓ PASS")

def main():
    """Run all tests"""
    print("\n=== Array Operators Integration Tests ===\n")

    tests = [
        test_push_simple,
        test_push_each,
        test_pull_simple,
        test_pull_with_condition,
        test_addtoset_simple,
        test_addtoset_duplicate,
        test_pop_first,
        test_pop_last,
        test_combined_operations,
    ]

    passed = 0
    failed = 0

    for test in tests:
        try:
            test()
            passed += 1
        except AssertionError as e:
            print(f"  ✗ FAIL: {e}")
            failed += 1
        except Exception as e:
            print(f"  ✗ ERROR: {e}")
            failed += 1

    print(f"\n=== Summary ===")
    print(f"Passed: {passed}")
    print(f"Failed: {failed}")
    print(f"Total:  {passed + failed}\n")

    if failed > 0:
        exit(1)

if __name__ == "__main__":
    main()
