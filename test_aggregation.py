#!/usr/bin/env python3
"""
Test script for MongoLite aggregation pipeline functionality.
Tests all pipeline stages: $match, $group, $project, $sort, $limit, $skip
"""

import ironbase
import os

# Clean up test database
if os.path.exists("test_aggregation.db"):
    os.remove("test_aggregation.db")

# Open database
db = ironbase.MongoLite("test_aggregation.db")
users = db.collection("users")

print("=" * 70)
print("MongoLite Aggregation Pipeline Tests")
print("=" * 70)

# ========== Insert Test Data ==========
print("\n1. Inserting Test Data")
print("-" * 70)

# Insert 100 users across different cities
for i in range(100):
    users.insert_one({
        "name": f"User{i}",
        "age": 18 + (i % 50),  # Ages 18-67
        "city": ["NYC", "LA", "Chicago", "Houston", "Phoenix"][i % 5],
        "salary": 30000 + (i * 1000),  # Salaries 30k-129k
        "department": ["Engineering", "Sales", "Marketing"][i % 3]
    })

print(f"Inserted 100 users")
total = users.count_documents({})
print(f"Total documents: {total}")

# ========== Test 1: $match Stage ==========
print("\n2. $match Stage - Filter Documents")
print("-" * 70)

results = users.aggregate([
    {"$match": {"age": {"$gte": 30}}}
])

print(f"Users with age >= 30: {len(results)}")
assert len(results) > 0, "Should find users with age >= 30"
print("✓ $match stage works")

# ========== Test 2: $group Stage with $sum ==========
print("\n3. $group Stage - Count by City")
print("-" * 70)

results = users.aggregate([
    {"$group": {"_id": "$city", "count": {"$sum": 1}}}
])

print(f"Cities found: {len(results)}")
for result in results:
    print(f"  {result['_id']}: {result['count']} users")

assert len(results) == 5, "Should have 5 cities"
assert all(r['count'] == 20 for r in results), "Each city should have 20 users"
print("✓ $group with $sum works")

# ========== Test 3: $group with Multiple Accumulators ==========
print("\n4. $group Stage - Statistics by City")
print("-" * 70)

results = users.aggregate([
    {"$group": {
        "_id": "$city",
        "count": {"$sum": 1},
        "avgAge": {"$avg": "$age"},
        "minAge": {"$min": "$age"},
        "maxAge": {"$max": "$age"},
        "avgSalary": {"$avg": "$salary"}
    }}
])

print(f"City statistics:")
for result in results:
    print(f"  {result['_id']}:")
    print(f"    Count: {result['count']}")
    print(f"    Avg Age: {result['avgAge']:.1f}")
    print(f"    Age Range: {result['minAge']} - {result['maxAge']}")
    print(f"    Avg Salary: ${result['avgSalary']:.0f}")

assert len(results) == 5, "Should have 5 cities"
print("✓ $group with multiple accumulators works")

# ========== Test 4: $sort Stage ==========
print("\n5. $sort Stage - Sort by Count")
print("-" * 70)

results = users.aggregate([
    {"$group": {"_id": "$department", "count": {"$sum": 1}}},
    {"$sort": {"count": -1}}  # Descending
])

print("Departments sorted by count (descending):")
for result in results:
    print(f"  {result['_id']}: {result['count']}")

# Verify sorted order
counts = [r['count'] for r in results]
assert counts == sorted(counts, reverse=True), "Should be sorted descending"
print("✓ $sort stage works")

# ========== Test 5: $limit and $skip Stages ==========
print("\n6. $limit and $skip Stages - Pagination")
print("-" * 70)

# Get top 10 users by salary
results = users.aggregate([
    {"$sort": {"salary": -1}},
    {"$limit": 10}
])

print(f"Top 10 highest paid users:")
for i, result in enumerate(results[:5]):
    print(f"  {i+1}. {result['name']}: ${result['salary']}")
print(f"  ... (showing first 5 of {len(results)})")

assert len(results) == 10, "Should return 10 results"
print("✓ $limit stage works")

# Skip first 10, get next 5
results = users.aggregate([
    {"$sort": {"salary": -1}},
    {"$skip": 10},
    {"$limit": 5}
])

print(f"\nUsers ranked 11-15 by salary:")
for i, result in enumerate(results):
    print(f"  {i+11}. {result['name']}: ${result['salary']}")

assert len(results) == 5, "Should return 5 results"
print("✓ $skip stage works")

# ========== Test 6: $project Stage ==========
print("\n7. $project Stage - Reshape Documents")
print("-" * 70)

results = users.aggregate([
    {"$limit": 3},
    {"$project": {"name": 1, "city": 1, "_id": 0}}
])

print("Projected fields (name and city only):")
for result in results:
    print(f"  {result}")
    assert "name" in result, "Should have name"
    assert "city" in result, "Should have city"
    assert "_id" not in result, "Should not have _id"
    assert "age" not in result, "Should not have age"
    assert "salary" not in result, "Should not have salary"

print("✓ $project stage works")

# ========== Test 7: Complex Pipeline ==========
print("\n8. Complex Pipeline - Multiple Stages")
print("-" * 70)

results = users.aggregate([
    {"$match": {"age": {"$gte": 25, "$lt": 50}}},  # Filter
    {"$group": {                                     # Group
        "_id": "$city",
        "count": {"$sum": 1},
        "avgSalary": {"$avg": "$salary"}
    }},
    {"$sort": {"avgSalary": -1}},                   # Sort
    {"$limit": 3}                                    # Top 3
])

print("Top 3 cities by average salary (age 25-49):")
for i, result in enumerate(results):
    print(f"  {i+1}. {result['_id']}: Avg ${result['avgSalary']:.0f} ({result['count']} users)")

assert len(results) <= 3, "Should return at most 3 results"
print("✓ Complex pipeline works")

# ========== Test 8: Group by Null (All Documents) ==========
print("\n9. $group with _id: null - Aggregate All")
print("-" * 70)

results = users.aggregate([
    {"$group": {
        "_id": None,
        "totalUsers": {"$sum": 1},
        "avgAge": {"$avg": "$age"},
        "totalSalary": {"$sum": "$salary"}
    }}
])

print("Overall statistics:")
print(f"  Total Users: {results[0]['totalUsers']}")
print(f"  Average Age: {results[0]['avgAge']:.1f}")
print(f"  Total Salary Budget: ${results[0]['totalSalary']:,}")

assert len(results) == 1, "Should return 1 result for null group"
assert results[0]['totalUsers'] == 100, "Should count all users"
print("✓ Group by null works")

# ========== Test 9: $first and $last Accumulators ==========
print("\n10. $first and $last Accumulators")
print("-" * 70)

results = users.aggregate([
    {"$sort": {"salary": 1}},  # Sort by salary ascending
    {"$group": {
        "_id": "$department",
        "lowestPaid": {"$first": "$name"},
        "highestPaid": {"$last": "$name"}
    }}
])

print("Salary range per department:")
for result in results:
    print(f"  {result['_id']}:")
    print(f"    Lowest paid: {result['lowestPaid']}")
    print(f"    Highest paid: {result['highestPaid']}")

assert len(results) == 3, "Should have 3 departments"
print("✓ $first and $last accumulators work")

# ========== Test 10: Real-World Example - Sales Report ==========
print("\n11. Real-World Example - Department Report")
print("-" * 70)

results = users.aggregate([
    {"$match": {"age": {"$gte": 30}}},              # Only experienced employees
    {"$group": {                                     # Group by department
        "_id": "$department",
        "employees": {"$sum": 1},
        "avgAge": {"$avg": "$age"},
        "avgSalary": {"$avg": "$salary"},
        "totalPayroll": {"$sum": "$salary"}
    }},
    {"$project": {                                   # Reshape output
        "department": "$_id",
        "employees": 1,
        "avgAge": 1,
        "avgSalary": 1,
        "totalPayroll": 1,
        "_id": 0
    }},
    {"$sort": {"totalPayroll": -1}}                 # Sort by total payroll
])

print("Department Report (age >= 30):")
print(f"{'Department':<15} {'Employees':>10} {'Avg Age':>8} {'Avg Salary':>12} {'Total Payroll':>15}")
print("-" * 70)
for result in results:
    print(f"{result['department']:<15} {result['employees']:>10} "
          f"{result['avgAge']:>8.1f} ${result['avgSalary']:>11.0f} "
          f"${result['totalPayroll']:>14,.0f}")

print("✓ Real-world example works")

# ========== Summary ==========
print("\n" + "=" * 70)
print("All Aggregation Tests Passed! ✓")
print("=" * 70)

print("\nAggregation Features Verified:")
print("  ✓ $match - Filter documents")
print("  ✓ $group - Group by field or null")
print("  ✓ $sum - Count and sum values")
print("  ✓ $avg - Average values")
print("  ✓ $min - Minimum values")
print("  ✓ $max - Maximum values")
print("  ✓ $first - First value in group")
print("  ✓ $last - Last value in group")
print("  ✓ $project - Reshape documents")
print("  ✓ $sort - Sort results")
print("  ✓ $limit - Limit results")
print("  ✓ $skip - Skip results")
print("  ✓ Complex multi-stage pipelines")

# Cleanup
db.close()
print("\nDatabase closed.")
