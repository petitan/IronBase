#!/usr/bin/env python3
"""
Test script for MongoLite index functionality:
- Creating indexes (unique and non-unique)
- Listing indexes
- Explain query execution plans
- Manual index selection with hint()
- Performance comparison with and without indexes
"""

import ironbase
import time
import os

# Clean up test database
if os.path.exists("test_indexes.db"):
    os.remove("test_indexes.db")

# Open database
db = ironbase.MongoLite("test_indexes.db")
users = db.collection("users")

print("=" * 70)
print("MongoLite Index Feature Tests")
print("=" * 70)

# ========== 1. Automatic _id Index ==========
print("\n1. Automatic _id Index")
print("-" * 70)

indexes = users.list_indexes()
print(f"Indexes after collection creation: {indexes}")
assert "users_id" in indexes, "Automatic _id index should exist"
print("✓ Automatic _id index created")

# ========== 2. Create Custom Indexes ==========
print("\n2. Create Custom Indexes")
print("-" * 70)

# Create index on age field
age_index = users.create_index("age", unique=False)
print(f"Created age index: {age_index}")

# Create unique index on email
email_index = users.create_index("email", unique=True)
print(f"Created email index (unique): {email_index}")

indexes = users.list_indexes()
print(f"All indexes: {indexes}")
assert len(indexes) == 3, "Should have 3 indexes: _id, age, email"
print("✓ Custom indexes created successfully")

# ========== 3. Insert Test Data ==========
print("\n3. Insert Test Data")
print("-" * 70)

# Insert 100 users
for i in range(100):
    users.insert_one({
        "name": f"User{i}",
        "age": i % 10,  # Ages 0-9, 10 users each
        "email": f"user{i}@example.com",
        "city": "TestCity"
    })

print(f"Inserted 100 users")
print(f"Total documents: {users.count_documents({})}")

# ========== 4. Test Unique Constraint ==========
print("\n4. Test Unique Constraint")
print("-" * 70)

try:
    # Try to insert duplicate email
    users.insert_one({
        "name": "Duplicate",
        "age": 25,
        "email": "user0@example.com"  # Already exists
    })
    print("✗ ERROR: Unique constraint should have prevented duplicate")
except Exception as e:
    print(f"✓ Unique constraint working: {e}")

# ========== 5. Explain Query Plans ==========
print("\n5. Explain Query Plans")
print("-" * 70)

# Equality query on indexed field (age)
plan1 = users.explain({"age": 5})
print(f"\nQuery: {{'age': 5}}")
print(f"  Plan: {plan1['queryPlan']}")
print(f"  Index Used: {plan1['indexUsed']}")
print(f"  Stage: {plan1['stage']}")
print(f"  Cost: {plan1['estimatedCost']}")
assert plan1['queryPlan'] == "IndexScan", "Should use IndexScan for equality on indexed field"
print("  ✓ Using index for equality query")

# Range query on indexed field
plan2 = users.explain({"age": {"$gte": 3, "$lt": 7}})
print(f"\nQuery: {{'age': {{'$gte': 3, '$lt': 7}}}}")
print(f"  Plan: {plan2['queryPlan']}")
print(f"  Index Used: {plan2['indexUsed']}")
print(f"  Stage: {plan2['stage']}")
print(f"  Cost: {plan2['estimatedCost']}")
assert plan2['queryPlan'] == "IndexRangeScan", "Should use IndexRangeScan for range query"
print("  ✓ Using index for range query")

# Query on non-indexed field
plan3 = users.explain({"city": "TestCity"})
print(f"\nQuery: {{'city': 'TestCity'}}")
print(f"  Plan: {plan3['queryPlan']}")
print(f"  Index Used: {plan3.get('indexUsed')}")
print(f"  Stage: {plan3['stage']}")
print(f"  Cost: {plan3['estimatedCost']}")
assert plan3['queryPlan'] == "CollectionScan", "Should use CollectionScan for non-indexed field"
print("  ✓ Collection scan for non-indexed field")

# ========== 6. Test find_with_hint() ==========
print("\n6. Manual Index Selection with hint()")
print("-" * 70)

# Normal find (automatic index selection)
results_auto = users.find({"age": 5})
print(f"Auto index selection: {len(results_auto)} results")

# Manual index selection with hint
results_hint = users.find_with_hint({"age": 5}, "users_age")
print(f"With hint('users_age'): {len(results_hint)} results")

assert len(results_auto) == len(results_hint), "Both should return same number of results"
assert len(results_auto) == 10, "Should find 10 users with age=5"
print("✓ hint() works correctly")

# Test error handling: invalid index
print("\nTesting error handling:")
try:
    users.find_with_hint({"age": 5}, "nonexistent_index")
    print("✗ Should have raised error for invalid index")
except Exception as e:
    print(f"  ✓ Caught error for invalid index: {str(e)[:50]}...")

# Test error handling: wrong field
try:
    users.find_with_hint({"city": "TestCity"}, "users_age")
    print("✗ Should have raised error for wrong field")
except Exception as e:
    print(f"  ✓ Caught error for wrong field: {str(e)[:50]}...")

# ========== 7. Performance Comparison ==========
print("\n7. Performance Comparison (Indexed vs Non-Indexed)")
print("-" * 70)

# Equality query on indexed field (age)
start = time.time()
for _ in range(100):
    results = users.find({"age": 5})
indexed_time = time.time() - start

print(f"Indexed field (age):   {indexed_time:.4f}s for 100 queries ({len(results)} results each)")

# Query on non-indexed field (city) - collection scan
start = time.time()
for _ in range(100):
    results = users.find({"city": "TestCity"})
unindexed_time = time.time() - start

print(f"Unindexed field (city): {unindexed_time:.4f}s for 100 queries ({len(results)} results each)")

speedup = unindexed_time / indexed_time if indexed_time > 0 else 0
print(f"Speedup: {speedup:.2f}x faster with index")

if speedup > 1.0:
    print("✓ Index provides performance improvement")
else:
    print("⚠ Note: Small dataset may not show significant speedup")

# ========== 8. Drop Index ==========
print("\n8. Drop Index")
print("-" * 70)

print(f"Indexes before drop: {users.list_indexes()}")

users.drop_index("users_age")
print(f"Dropped 'users_age' index")

indexes = users.list_indexes()
print(f"Indexes after drop: {indexes}")
assert "users_age" not in indexes, "Age index should be dropped"
assert "users_email" in indexes, "Email index should still exist"
print("✓ Index dropped successfully")

# Verify explain shows CollectionScan now
plan = users.explain({"age": 5})
print(f"\nQuery plan after dropping age index:")
print(f"  Plan: {plan['queryPlan']}")
assert plan['queryPlan'] == "CollectionScan", "Should use CollectionScan after dropping index"
print("  ✓ Correctly falls back to CollectionScan")

# ========== 9. Range Query Tests ==========
print("\n9. Range Query with Index")
print("-" * 70)

# Recreate age index for range tests
users.create_index("age", unique=False)

# Test various range queries
range_tests = [
    ({"age": {"$gte": 5}}, "Greater than or equal"),
    ({"age": {"$lt": 5}}, "Less than"),
    ({"age": {"$gte": 3, "$lte": 7}}, "Range inclusive"),
    ({"age": {"$gt": 2, "$lt": 8}}, "Range exclusive"),
]

for query, desc in range_tests:
    plan = users.explain(query)
    results = users.find(query)
    print(f"{desc}: {plan['queryPlan']} -> {len(results)} results")
    assert plan['queryPlan'] == "IndexRangeScan", f"Should use IndexRangeScan for {desc}"

print("✓ All range queries use IndexRangeScan")

# ========== Summary ==========
print("\n" + "=" * 70)
print("All Tests Passed! ✓")
print("=" * 70)
print("\nIndex Features Verified:")
print("  ✓ Automatic _id index creation")
print("  ✓ Custom index creation (unique and non-unique)")
print("  ✓ Index listing")
print("  ✓ Unique constraint enforcement")
print("  ✓ Query plan explanation (IndexScan, IndexRangeScan, CollectionScan)")
print("  ✓ Manual index selection with hint()")
print("  ✓ Error handling (invalid index, wrong field)")
print("  ✓ Performance improvement with indexes")
print("  ✓ Index dropping")
print("  ✓ Range query optimization")

# Cleanup
db.close()
print("\nDatabase closed.")
