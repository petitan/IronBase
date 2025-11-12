#!/usr/bin/env python3
"""Debug find() deserialization issue"""

from ironbase import IronBase
import os

# Clean up
for f in ["test_find.mlite", "test_find.wal"]:
    if os.path.exists(f):
        os.remove(f)

print("=" * 60)
print("Creating database and inserting documents")
db = IronBase("test_find.mlite")
products = db.collection("products")

# Insert 5 documents
docs = [
    {"name": "Product A", "category": "Electronics", "price": 100},
    {"name": "Product B", "category": "Books", "price": 20},
    {"name": "Product C", "category": "Electronics", "price": 150},
    {"name": "Product D", "category": "Clothing", "price": 50},
    {"name": "Product E", "category": "Electronics", "price": 200},
]

result = products.insert_many(docs)
print(f"✓ Inserted {result['inserted_count']} documents")

# Test count (this works)
count = products.count_documents({})
print(f"✓ Count: {count}")

print("\n" + "=" * 60)
print("Testing find() queries")

# Test 1: Find all
print("\nTest 1: Find all documents")
try:
    all_docs = list(products.find({}))
    print(f"✓ Found {len(all_docs)} documents")
    for doc in all_docs:
        print(f"  - {doc.get('name')}: {doc.get('category')}")
except Exception as e:
    print(f"❌ Error: {e}")
    import traceback
    traceback.print_exc()

# Test 2: Find one
print("\nTest 2: Find one document")
try:
    doc = products.find_one({})
    print(f"✓ Found: {doc}")
except Exception as e:
    print(f"❌ Error: {e}")
    import traceback
    traceback.print_exc()

# Test 3: Find with filter
print("\nTest 3: Find with category filter")
try:
    electronics = list(products.find({"category": "Electronics"}))
    print(f"✓ Found {len(electronics)} electronics")
    for doc in electronics:
        print(f"  - {doc.get('name')}: ${doc.get('price')}")
except Exception as e:
    print(f"❌ Error: {e}")
    import traceback
    traceback.print_exc()

# Cleanup
db.close()
os.remove("test_find.mlite")
os.remove("test_find.wal")
print("\n✓ Test completed")
