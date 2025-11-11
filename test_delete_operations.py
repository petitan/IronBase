#!/usr/bin/env python3
"""
Test DELETE operations with tombstone pattern
"""
import ironbase
import os

# Clean up
if os.path.exists("test_delete.mlite"):
    os.remove("test_delete.mlite")

print("=" * 60)
print("TEST: DELETE Operations with Tombstone Pattern")
print("=" * 60)

db = ironbase.MongoLite("test_delete.mlite")
collection = db.collection("users")

# Insert test documents
print("\n📝 Inserting test documents...")
collection.insert_one({"_id": 1, "name": "Alice", "age": 30, "city": "NYC"})
collection.insert_one({"_id": 2, "name": "Bob", "age": 25, "city": "LA"})
collection.insert_one({"_id": 3, "name": "Carol", "age": 35, "city": "NYC"})
collection.insert_one({"_id": 4, "name": "Dave", "age": 28, "city": "SF"})
print(f"   ✅ Inserted 4 documents")

# Verify insert
count = collection.count_documents({})
print(f"   Initial count: {count}")
assert count == 4, f"Expected 4, got {count}"

# Test 1: Delete by _id
print("\n🗑️ Test 1: DELETE by _id")
result = collection.delete_one({"_id": 2})
print(f"   Deleted: {result['deleted_count']}")
assert result["deleted_count"] == 1, f"Expected deleted=1, got {result['deleted_count']}"

# Verify deletion
count = collection.count_documents({})
print(f"   Count after delete: {count}")
assert count == 3, f"Expected 3, got {count}"

# Verify Bob is gone
bob = collection.find_one({"_id": 2})
assert bob is None, "Bob should be deleted"
print("   ✅ Delete by _id WORKS! Bob is gone.")

# Test 2: Delete by field
print("\n🗑️ Test 2: DELETE by field (name)")
result = collection.delete_one({"name": "Dave"})
print(f"   Deleted: {result['deleted_count']}")
assert result["deleted_count"] == 1, f"Expected deleted=1, got {result['deleted_count']}"

count = collection.count_documents({})
print(f"   Count after delete: {count}")
assert count == 2, f"Expected 2, got {count}"
print("   ✅ Delete by field WORKS!")

# Test 3: Delete many by city
print("\n🗑️ Test 3: DELETE MANY by city (NYC)")
result = collection.delete_many({"city": "NYC"})
print(f"   Deleted: {result['deleted_count']}")
assert result["deleted_count"] == 2, f"Expected deleted=2, got {result['deleted_count']}"

count = collection.count_documents({})
print(f"   Count after delete many: {count}")
assert count == 0, f"Expected 0, got {count}"
print("   ✅ Delete many WORKS! All documents deleted.")

# Test 4: Persistence after reopen
print("\n🔄 Test 4: PERSISTENCE after reopen")
db.close()

db2 = ironbase.MongoLite("test_delete.mlite")
collection2 = db2.collection("users")

count = collection2.count_documents({})
print(f"   Count after reopen: {count}")
assert count == 0, f"Expected 0, got {count}"

# Verify all deleted
all_docs = collection2.find({})
print(f"   Documents found: {len(all_docs)}")
assert len(all_docs) == 0, "Should have no documents"
print("   ✅ Deletions PERSISTED correctly!")

# Test 5: Insert after delete (new auto-generated IDs)
print("\n📝 Test 5: INSERT after delete (new IDs)")
result1 = collection2.insert_one({"name": "New Alice", "age": 25})
result2 = collection2.insert_one({"name": "New Bob", "age": 30})
print(f"   ✅ Inserted 2 new documents (IDs: {result1['inserted_id']}, {result2['inserted_id']})")

count = collection2.count_documents({})
print(f"   Count: {count}")
assert count == 2, f"Expected 2, got {count}"

# Verify new documents
alice = collection2.find_one({"name": "New Alice"})
bob = collection2.find_one({"name": "New Bob"})
assert alice is not None, "New Alice should exist"
assert bob is not None, "New Bob should exist"
print(f"   New Alice: {alice['name']}, age={alice['age']}")
print(f"   New Bob: {bob['name']}, age={bob['age']}")
assert alice["name"] == "New Alice", "Should be new Alice"
assert alice["age"] == 25, "Should be new age"
print("   ✅ New documents work after delete!")

db2.close()

print("\n" + "=" * 60)
print("✅ ALL DELETE TESTS PASSED!")
print("=" * 60)
