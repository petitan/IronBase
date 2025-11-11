#!/usr/bin/env python3
"""
Test compaction (garbage collection) functionality
"""
import ironbase
import os

# Clean up
if os.path.exists("test_compaction.mlite"):
    os.remove("test_compaction.mlite")

print("=" * 60)
print("TEST: Compaction (Garbage Collection)")
print("=" * 60)

db = ironbase.MongoLite("test_compaction.mlite")
collection = db.collection("users")

# Insert test documents
print("\n📝 Phase 1: Insert 100 documents...")
for i in range(100):
    collection.insert_one({"_id": i, "name": f"User{i}", "age": 20 + (i % 50)})
print(f"   ✅ Inserted 100 documents")

# Get initial file size
initial_size = os.path.getsize("test_compaction.mlite")
print(f"   Initial file size: {initial_size:,} bytes")

count = collection.count_documents({})
assert count == 100, f"Expected 100, got {count}"

# Delete 50 documents (creates tombstones)
print("\n🗑️ Phase 2: Delete 50 documents (creates tombstones)...")
deleted = collection.delete_many({"age": {"$lt": 35}})
print(f"   Deleted: {deleted['deleted_count']} documents")

count_after_delete = collection.count_documents({})
print(f"   Remaining documents: {count_after_delete}")

# File size should be similar (tombstones still take space)
size_after_delete = os.path.getsize("test_compaction.mlite")
print(f"   File size after delete: {size_after_delete:,} bytes")
assert size_after_delete >= initial_size * 0.9, "File size should not shrink much (tombstones)"

# Compact the database
print("\n🗜️ Phase 3: Run compaction...")
stats = db.compact()
print(f"   Size before: {stats['size_before']:,} bytes")
print(f"   Size after: {stats['size_after']:,} bytes")
print(f"   Space saved: {stats['space_saved']:,} bytes ({stats['space_saved'] / stats['size_before'] * 100:.1f}%)")
print(f"   Documents scanned: {stats['documents_scanned']}")
print(f"   Documents kept: {stats['documents_kept']}")
print(f"   Tombstones removed: {stats['tombstones_removed']}")

# Verify compaction
size_after_compact = os.path.getsize("test_compaction.mlite")
print(f"   ✅ File size after compact: {size_after_compact:,} bytes")

# File should be smaller after compaction
assert size_after_compact < size_after_delete, "File should shrink after compaction"
assert stats['tombstones_removed'] > 0, "Should have removed tombstones"
assert stats['documents_kept'] == count_after_delete, f"Should keep {count_after_delete} docs, kept {stats['documents_kept']}"

# Verify data integrity after compaction
print("\n🔍 Phase 4: Verify data integrity after compaction...")
count_final = collection.count_documents({})
assert count_final == count_after_delete, f"Count mismatch: expected {count_after_delete}, got {count_final}"
print(f"   ✅ Document count correct: {count_final}")

# Verify specific documents
doc50 = collection.find_one({"_id": 50})
assert doc50 is not None, "Doc 50 should exist"
expected_age = 20 + (50 % 50)  # age = 20
print(f"   ✅ Document 50 found: {doc50['name']}, age={doc50['age']}")

doc10 = collection.find_one({"_id": 10})
assert doc10 is None, "Doc 10 should be deleted (age 30)"
print(f"   ✅ Document 10 correctly deleted")

# Verify all remaining documents
remaining_docs = collection.find({})
assert len(remaining_docs) == count_after_delete, "All remaining docs should be findable"
print(f"   ✅ All {count_after_delete} documents accessible")

# Persistence after compact and reopen
print("\n🔄 Phase 5: Test persistence after compact...")
db.close()

db2 = ironbase.MongoLite("test_compaction.mlite")
collection2 = db2.collection("users")
count_reopened = collection2.count_documents({})
assert count_reopened == count_after_delete, f"Count after reopen: expected {count_after_delete}, got {count_reopened}"
print(f"   ✅ Count after reopen: {count_reopened}")

# Verify doc still exists
doc50_after = collection2.find_one({"_id": 50})
assert doc50_after is not None, "Doc 50 should persist"
print(f"   ✅ Document 50 persisted: {doc50_after['name']}, age={doc50_after['age']}")

db2.close()

print("\n" + "=" * 60)
print("✅ ALL COMPACTION TESTS PASSED!")
print("=" * 60)
print(f"\nCompaction efficiency:")
print(f"  • Initial size: {initial_size:,} bytes")
print(f"  • After deletes: {size_after_delete:,} bytes")
print(f"  • After compact: {size_after_compact:,} bytes")
print(f"  • Space reclaimed: {size_after_delete - size_after_compact:,} bytes")
print(f"  • Reduction: {(1 - size_after_compact / size_after_delete) * 100:.1f}%")
