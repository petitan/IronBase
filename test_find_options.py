#!/usr/bin/env python3
"""
Test script for MongoLite find() options: projection, sort, limit, skip
"""

import ironbase
import os

# Clean up test database
if os.path.exists("test_find_options.db"):
    os.remove("test_find_options.db")

# Open database
db = ironbase.MongoLite("test_find_options.db")
users = db.collection("users")

print("=" * 70)
print("MongoLite Find Options Tests")
print("=" * 70)

# ========== Insert Test Data ==========
print("\n1. Inserting Test Data")
print("-" * 70)

users.insert_many([
    {"name": "Alice", "age": 30, "city": "NYC", "salary": 80000},
    {"name": "Bob", "age": 25, "city": "LA", "salary": 70000},
    {"name": "Carol", "age": 35, "city": "NYC", "salary": 95000},
    {"name": "David", "age": 28, "city": "Chicago", "salary": 75000},
    {"name": "Eve", "age": 32, "city": "LA", "salary": 85000},
    {"name": "Frank", "age": 40, "city": "NYC", "salary": 100000},
    {"name": "Grace", "age": 27, "city": "Chicago", "salary": 72000},
    {"name": "Henry", "age": 33, "city": "LA", "salary": 88000},
])

print(f"Inserted {users.count_documents({})} users")

# ========== Test 1: Projection (Include Mode) ==========
print("\n2. Projection - Include Mode")
print("-" * 70)

results = users.find({}, projection={"name": 1, "age": 1, "_id": 0})

print("Fields: name, age (excluding _id)")
for user in results[:3]:
    print(f"  {user}")
    assert "name" in user, "Should have name"
    assert "age" in user, "Should have age"
    assert "_id" not in user, "Should not have _id"
    assert "city" not in user, "Should not have city"
    assert "salary" not in user, "Should not have salary"

print("✓ Projection include mode works")

# ========== Test 2: Projection (Exclude Mode) ==========
print("\n3. Projection - Exclude Mode")
print("-" * 70)

results = users.find({}, projection={"salary": 0, "_collection": 0})

print("Exclude: salary, _collection")
for user in results[:2]:
    print(f"  {user}")
    assert "name" in user, "Should have name"
    assert "age" in user, "Should have age"
    assert "_id" in user, "Should have _id"
    assert "city" in user, "Should have city"
    assert "salary" not in user, "Should not have salary"

print("✓ Projection exclude mode works")

# ========== Test 3: Sort (Single Field, Ascending) ==========
print("\n4. Sort - Single Field Ascending")
print("-" * 70)

results = users.find({}, sort=[("age", 1)])

print("Sorted by age (ascending):")
ages = [r["age"] for r in results]
for i, user in enumerate(results[:4]):
    print(f"  {i+1}. {user['name']}: age {user['age']}")

assert ages == sorted(ages), "Ages should be in ascending order"
print("✓ Sort ascending works")

# ========== Test 4: Sort (Single Field, Descending) ==========
print("\n5. Sort - Single Field Descending")
print("-" * 70)

results = users.find({}, sort=[("salary", -1)])

print("Sorted by salary (descending):")
salaries = [r["salary"] for r in results]
for i, user in enumerate(results[:4]):
    print(f"  {i+1}. {user['name']}: ${user['salary']}")

assert salaries == sorted(salaries, reverse=True), "Salaries should be in descending order"
print("✓ Sort descending works")

# ========== Test 5: Sort (Multi-Field) ==========
print("\n6. Sort - Multi-Field")
print("-" * 70)

results = users.find({}, sort=[("city", 1), ("age", -1)])

print("Sorted by city (asc), then age (desc):")
for user in results:
    print(f"  {user['city']:<10} {user['name']:<10} age {user['age']}")

# Verify sorting
cities = [r["city"] for r in results]
prev_city = None
prev_age = None
for user in results:
    curr_city = user["city"]
    curr_age = user["age"]

    if prev_city == curr_city and prev_age is not None:
        # Within same city, ages should be descending
        assert curr_age <= prev_age, f"Ages should be descending within {curr_city}"

    prev_city = curr_city
    prev_age = curr_age

print("✓ Multi-field sort works")

# ========== Test 6: Limit ==========
print("\n7. Limit")
print("-" * 70)

results = users.find({}, limit=3)

print(f"Requested limit=3, got {len(results)} results:")
for user in results:
    print(f"  {user['name']}")

assert len(results) == 3, "Should return exactly 3 results"
print("✓ Limit works")

# ========== Test 7: Skip ==========
print("\n8. Skip")
print("-" * 70)

all_users = users.find({}, sort=[("name", 1)])
skipped_users = users.find({}, sort=[("name", 1)], skip=2)

print(f"All users: {[u['name'] for u in all_users]}")
print(f"Skip first 2: {[u['name'] for u in skipped_users]}")

assert len(skipped_users) == len(all_users) - 2, "Should skip 2 users"
assert skipped_users[0]["name"] == all_users[2]["name"], "First result should be 3rd user"
print("✓ Skip works")

# ========== Test 8: Pagination (Skip + Limit) ==========
print("\n9. Pagination (Skip + Limit)")
print("-" * 70)

page_size = 3

# Page 1
page1 = users.find({}, sort=[("name", 1)], limit=page_size, skip=0)
print(f"Page 1 (limit={page_size}, skip=0): {[u['name'] for u in page1]}")

# Page 2
page2 = users.find({}, sort=[("name", 1)], limit=page_size, skip=page_size)
print(f"Page 2 (limit={page_size}, skip={page_size}): {[u['name'] for u in page2]}")

# Page 3
page3 = users.find({}, sort=[("name", 1)], limit=page_size, skip=page_size * 2)
print(f"Page 3 (limit={page_size}, skip={page_size * 2}): {[u['name'] for u in page3]}")

assert len(page1) == page_size, "Page 1 should have 3 results"
assert len(page2) == page_size, "Page 2 should have 3 results"
assert len(page3) == 2, "Page 3 should have 2 results (only 8 total)"

# Verify no overlap
all_names = set()
for page in [page1, page2, page3]:
    for user in page:
        assert user["name"] not in all_names, "No duplicate names across pages"
        all_names.add(user["name"])

print("✓ Pagination works")

# ========== Test 9: Combined (Query + Projection + Sort + Limit) ==========
print("\n10. Combined: Query + Projection + Sort + Limit")
print("-" * 70)

results = users.find(
    {"age": {"$gte": 30}},           # Query: age >= 30
    projection={"name": 1, "age": 1, "salary": 1, "_id": 0},
    sort=[("salary", -1)],            # Sort by salary descending
    limit=3                           # Top 3
)

print("Top 3 highest-paid users aged 30+:")
for i, user in enumerate(results):
    print(f"  {i+1}. {user['name']}: age {user['age']}, salary ${user['salary']}")
    assert user["age"] >= 30, "Age should be >= 30"
    assert "_id" not in user, "Should not have _id"
    assert "city" not in user, "Should not have city"

assert len(results) == 3, "Should return 3 results"

# Verify they're sorted by salary descending
salaries = [u["salary"] for u in results]
assert salaries == sorted(salaries, reverse=True), "Should be sorted by salary desc"

print("✓ Combined query + options works")

# ========== Test 10: Empty Results with Options ==========
print("\n11. Empty Results with Options")
print("-" * 70)

results = users.find(
    {"age": {"$gt": 100}},  # No match
    projection={"name": 1},
    sort=[("age", 1)],
    limit=10
)

print(f"Query with no matches: {len(results)} results")
assert len(results) == 0, "Should return empty list"
print("✓ Empty results handled correctly")

# ========== Test 11: Sort by String Field ==========
print("\n12. Sort by String Field")
print("-" * 70)

results = users.find({}, sort=[("name", 1)])

print("Users sorted by name (alphabetically):")
names = [r["name"] for r in results]
for name in names:
    print(f"  {name}")

assert names == sorted(names), "Names should be in alphabetical order"
print("✓ String sorting works")

# ========== Test 12: Projection with Query ==========
print("\n13. Projection with Filtered Query")
print("-" * 70)

results = users.find(
    {"city": "NYC"},
    projection={"name": 1, "city": 1, "_id": 0}
)

print("NYC users (name and city only):")
for user in results:
    print(f"  {user}")
    assert user["city"] == "NYC", "Should only return NYC users"
    assert "name" in user and "city" in user, "Should have name and city"
    assert "_id" not in user, "Should not have _id"

print("✓ Projection with query works")

# ========== Summary ==========
print("\n" + "=" * 70)
print("All Find Options Tests Passed! ✓")
print("=" * 70)

print("\nFeatures Verified:")
print("  ✓ Projection - include mode")
print("  ✓ Projection - exclude mode")
print("  ✓ Sort - single field ascending")
print("  ✓ Sort - single field descending")
print("  ✓ Sort - multi-field")
print("  ✓ Sort - string fields")
print("  ✓ Limit")
print("  ✓ Skip")
print("  ✓ Pagination (skip + limit)")
print("  ✓ Combined query + projection + sort + limit")
print("  ✓ Empty results handling")
print("  ✓ Projection with filtered query")

# Cleanup
db.close()
print("\nDatabase closed.")
