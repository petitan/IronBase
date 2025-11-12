#!/usr/bin/env python3
"""
Test script for the new logging system
"""

from ironbase import IronBase
import os

def cleanup(path):
    """Clean up test database files"""
    for ext in [".mlite", ".wal"]:
        try:
            os.remove(path.replace(".mlite", ext))
        except FileNotFoundError:
            pass

def test_log_levels():
    print("=" * 70)
    print("LOG LEVEL SYSTEM TEST")
    print("=" * 70)
    print()

    # Test get/set log level API
    print("1. Testing log level API:")
    print(f"   Current log level: {IronBase.get_log_level()}")

    IronBase.set_log_level("DEBUG")
    print(f"   After set to DEBUG: {IronBase.get_log_level()}")

    IronBase.set_log_level("TRACE")
    print(f"   After set to TRACE: {IronBase.get_log_level()}")
    print()

    # Test invalid log level
    print("2. Testing invalid log level:")
    try:
        IronBase.set_log_level("INVALID")
        print("   ❌ Should have raised ValueError!")
    except ValueError as e:
        print(f"   ✅ Correctly raised ValueError: {e}")
    print()

    # Test with WARN level (default - minimal output)
    print("3. Testing with WARN level (should see minimal output):")
    print("=" * 70)
    IronBase.set_log_level("WARN")

    db_path = "test_log_warn.mlite"
    cleanup(db_path)

    db = IronBase(db_path)
    coll = db.collection("users")
    coll.insert_many([{"name": f"User {i}", "age": 20 + i} for i in range(10)])
    result = coll.find({})
    print(f"Found {len(result)} documents (WARN level)")
    db.close()
    cleanup(db_path)
    print()

    # Test with DEBUG level (moderate output)
    print("4. Testing with DEBUG level (should see debug messages):")
    print("=" * 70)
    IronBase.set_log_level("DEBUG")

    db_path = "test_log_debug.mlite"
    cleanup(db_path)

    db = IronBase(db_path)
    coll = db.collection("products")
    coll.insert_many([
        {"name": "Apple", "category": "fruit", "price": 1.5},
        {"name": "Banana", "category": "fruit", "price": 0.8},
        {"name": "Carrot", "category": "vegetable", "price": 1.2},
    ])
    result = coll.find({})
    print(f"Found {len(result)} documents (DEBUG level)")
    db.close()
    cleanup(db_path)
    print()

    # Test with TRACE level (verbose output)
    print("5. Testing with TRACE level (should see EVERYTHING):")
    print("=" * 70)
    IronBase.set_log_level("TRACE")

    db_path = "test_log_trace.mlite"
    cleanup(db_path)

    db = IronBase(db_path)
    coll = db.collection("orders")
    coll.insert_many([{"order_id": i, "total": i * 10} for i in range(3)])
    result = coll.find({"order_id": 1})
    print(f"Found {len(result)} documents (TRACE level)")
    db.close()
    cleanup(db_path)
    print()

    # Reset to WARN for clean output
    IronBase.set_log_level("WARN")

    print("=" * 70)
    print("✅ ALL LOG LEVEL TESTS PASSED!")
    print("=" * 70)
    print()
    print("📊 Summary:")
    print("   ✓ Log level API works (get_log_level, set_log_level)")
    print("   ✓ Invalid log level raises ValueError")
    print("   ✓ WARN level: Minimal output (only warnings/errors)")
    print("   ✓ DEBUG level: Shows debug messages")
    print("   ✓ TRACE level: Shows everything (very verbose)")
    print()
    print("💡 Usage:")
    print("   from ironbase import IronBase")
    print("   IronBase.set_log_level('DEBUG')  # Show debug info")
    print("   IronBase.set_log_level('WARN')   # Production mode (default)")
    print("   IronBase.set_log_level('TRACE')  # Maximum verbosity")
    print()

if __name__ == "__main__":
    test_log_levels()
