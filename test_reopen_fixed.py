#!/usr/bin/env python3
"""Test database reopen to verify data persistence fix"""
import ironbase
import os

DB_FILE = 'test_reopen_fixed.db'

# Clean up
if os.path.exists(DB_FILE):
    os.remove(DB_FILE)

print("=" * 60)
print("Testing Database Reopen (Data Persistence Fix)")
print("=" * 60)

# ==================== Phase 1: Create and populate ====================
print("\nPhase 1: Creating database and inserting data")
db = ironbase.MongoLite(DB_FILE)
users = db.collection('users')

result = users.insert_many([
    {'name': 'Alice', 'age': 28, 'city': 'NYC'},
    {'name': 'Bob', 'age': 32, 'city': 'LA'},
    {'name': 'Carol', 'age': 25, 'city': 'SF'},
])

count1 = users.count_documents({})
print(f"✓ Inserted {len(result)} users")
print(f"✓ Count before close: {count1}")

# Get all IDs
all_docs = list(users.find({}))
ids_before = [doc['_id'] for doc in all_docs]
print(f"✓ IDs before close: {ids_before}")

db.close()
print("✓ Database closed")

# ==================== Phase 2: Reopen and verify ====================
print("\nPhase 2: Reopening database and verifying data")
db2 = ironbase.MongoLite(DB_FILE)
users2 = db2.collection('users')

count2 = users2.count_documents({})
print(f"Count after reopen: {count2}")

all_docs2 = list(users2.find({}))
ids_after = [doc['_id'] for doc in all_docs2]
print(f"IDs after reopen: {ids_after}")

# Show documents
print("\nDocuments after reopen:")
for doc in all_docs2:
    print(f"  • ID {doc['_id']}: {doc.get('name')} (age {doc.get('age')}, city {doc.get('city')})")

# ==================== Phase 3: Insert after reopen ====================
print("\nPhase 3: Inserting new document after reopen")
result3 = users2.insert_one({'name': 'Dave', 'age': 30, 'city': 'Boston'})
print(f"✓ Inserted Dave with ID: {result3}")

count3 = users2.count_documents({})
print(f"Count after new insert: {count3}")

all_docs3 = list(users2.find({}))
ids_final = [doc['_id'] for doc in all_docs3]
print(f"All IDs: {ids_final}")

print("\nAll documents:")
for doc in all_docs3:
    print(f"  • ID {doc['_id']}: {doc.get('name')} (age {doc.get('age')}, city {doc.get('city')})")

db2.close()

# ==================== Verification ====================
print("\n" + "=" * 60)
print("VERIFICATION")
print("=" * 60)

success = True

# Check: count should be 4 (3 original + 1 new)
if count3 != 4:
    print(f"❌ FAIL: Expected count 4, got {count3}")
    success = False
else:
    print(f"✓ Count is correct: {count3}")

# Check: IDs should be [1, 2, 3, 4]
if ids_final != [1, 2, 3, 4]:
    print(f"❌ FAIL: Expected IDs [1, 2, 3, 4], got {ids_final}")
    success = False
else:
    print(f"✓ IDs are correct: {ids_final}")

# Check: original data should be present
expected_names = {'Alice', 'Bob', 'Carol', 'Dave'}
actual_names = {doc['name'] for doc in all_docs3}
if actual_names != expected_names:
    print(f"❌ FAIL: Expected names {expected_names}, got {actual_names}")
    success = False
else:
    print(f"✓ All names present: {actual_names}")

print("\n" + "=" * 60)
if success:
    print("✅ Test PASSED! Data persistence works correctly!")
else:
    print("❌ Test FAILED!")
print("=" * 60)
