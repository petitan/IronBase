#!/usr/bin/env python3
"""
Test UPDATE operations after collection filter fix
"""
import ironbase
import os

# Clean up old test file
if os.path.exists("test_update.mlite"):
    os.remove("test_update.mlite")

print("=" * 60)
print("TEST: UPDATE Operations with Collection Filter Fix")
print("=" * 60)

# Create database and collection
db = ironbase.MongoLite("test_update.mlite")
collection = db.collection("users")

# Insert test documents
print("\n📝 Inserting test documents...")
collection.insert_one({"_id": 1, "name": "Alice", "age": 30, "city": "NYC"})
collection.insert_one({"_id": 2, "name": "Bob", "age": 25, "city": "LA"})
collection.insert_one({"_id": 3, "name": "Carol", "age": 35, "city": "NYC"})
print(f"   ✅ Inserted 3 documents")

# Verify insert
count = collection.count_documents({})
print(f"   Count: {count}")
assert count == 3, f"Expected 3, got {count}"

# Test 1: Update by _id (this was failing before)
print("\n🔧 Test 1: UPDATE by _id")
result = collection.update_one({"_id": 1}, {"$set": {"age": 31}})
print(f"   Matched: {result['matched_count']}, Modified: {result['modified_count']}")
assert result["matched_count"] == 1, f"Expected matched=1, got {result['matched_count']}"
assert result["modified_count"] == 1, f"Expected modified=1, got {result['modified_count']}"

# Verify update
doc = collection.find_one({"_id": 1})
print(f"   Updated doc: {doc}")
assert doc["age"] == 31, f"Expected age=31, got {doc['age']}"
print("   ✅ Update by _id WORKS!")

# Test 2: Update by field
print("\n🔧 Test 2: UPDATE by field")
result = collection.update_one({"name": "Bob"}, {"$set": {"age": 26}})
print(f"   Matched: {result['matched_count']}, Modified: {result['modified_count']}")
assert result["matched_count"] == 1, f"Expected matched=1, got {result['matched_count']}"
assert result["modified_count"] == 1, f"Expected modified=1, got {result['modified_count']}"

doc = collection.find_one({"_id": 2})
print(f"   Updated doc: {doc}")
assert doc["age"] == 26, f"Expected age=26, got {doc['age']}"
print("   ✅ Update by field WORKS!")

# Test 3: Update many
print("\n🔧 Test 3: UPDATE MANY by city")
result = collection.update_many({"city": "NYC"}, {"$set": {"country": "USA"}})
print(f"   Matched: {result['matched_count']}, Modified: {result['modified_count']}")
assert result["matched_count"] == 2, f"Expected matched=2, got {result['matched_count']}"
assert result["modified_count"] == 2, f"Expected modified=2, got {result['modified_count']}"

# Verify update_many
nyc_docs = collection.find({"city": "NYC"})
print(f"   NYC docs: {len(nyc_docs)}")
for doc in nyc_docs:
    assert "country" in doc, f"Missing 'country' field in {doc}"
    assert doc["country"] == "USA", f"Expected country=USA, got {doc['country']}"
    print(f"   - {doc['name']}: country={doc['country']}")
print("   ✅ Update many WORKS!")

# Test 4: Update operators ($inc, $push)
print("\n🔧 Test 4: UPDATE operators ($inc)")
result = collection.update_one({"_id": 3}, {"$inc": {"age": 1}})
print(f"   Matched: {result['matched_count']}, Modified: {result['modified_count']}")
assert result["matched_count"] == 1, f"Expected matched=1, got {result['matched_count']}"
assert result["modified_count"] == 1, f"Expected modified=1, got {result['modified_count']}"

doc = collection.find_one({"_id": 3})
print(f"   Updated doc: {doc}")
assert doc["age"] == 36, f"Expected age=36, got {doc['age']}"
print("   ✅ $inc operator WORKS!")

# Test 5: Persistence after reopen
print("\n🔄 Test 5: PERSISTENCE after reopen")
db.close()

db2 = ironbase.MongoLite("test_update.mlite")
collection2 = db2.collection("users")

count = collection2.count_documents({})
print(f"   Count after reopen: {count}")
assert count == 3, f"Expected 3, got {count}"

doc1 = collection2.find_one({"_id": 1})
doc2 = collection2.find_one({"_id": 2})
doc3 = collection2.find_one({"_id": 3})

print(f"   Doc 1 age: {doc1['age']} (expected 31)")
print(f"   Doc 2 age: {doc2['age']} (expected 26)")
print(f"   Doc 3 age: {doc3['age']} (expected 36)")

assert doc1["age"] == 31, "Doc 1 age mismatch"
assert doc2["age"] == 26, "Doc 2 age mismatch"
assert doc3["age"] == 36, "Doc 3 age mismatch"
print("   ✅ Updates PERSISTED correctly!")

db2.close()

print("\n" + "=" * 60)
print("✅ ALL UPDATE TESTS PASSED!")
print("=" * 60)
