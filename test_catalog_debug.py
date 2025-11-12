#!/usr/bin/env python3
"""Minimal test to debug catalog serialization"""

from ironbase import IronBase
import os

# Clean up
for f in ["test_catalog.mlite", "test_catalog.wal"]:
    if os.path.exists(f):
        os.remove(f)

print("=" * 60)
print("Step 1: Create database and insert 3 documents")
db = IronBase("test_catalog.mlite")
products = db.collection("products")

# Insert 3 documents
docs = [
    {"name": "Product A", "price": 10},
    {"name": "Product B", "price": 20},
    {"name": "Product C", "price": 30},
]
result = products.insert_many(docs)
print(f"✓ Inserted {result['inserted_count']} documents")

# Check count before close
count_before = products.count_documents({})
print(f"✓ Count before close: {count_before}")

print("\n" + "=" * 60)
print("Step 2: Close database (should flush metadata)")
db.close()

file_size = os.path.getsize("test_catalog.mlite")
print(f"✓ File size after close: {file_size} bytes")

print("\n" + "=" * 60)
print("Step 3: Reopen database and check catalog")
db2 = IronBase("test_catalog.mlite")
products2 = db2.collection("products")

count_after = products2.count_documents({})
print(f"✓ Count after reopen: {count_after}")

# Try to find documents
docs_found = list(products2.find({}))
print(f"✓ Documents found: {len(docs_found)}")

if count_after != 3:
    print(f"❌ ERROR: Expected 3 documents, found {count_after}")
    print("This means catalog was not persisted correctly!")
else:
    print("✅ SUCCESS: Catalog persisted correctly")

db2.close()

# Cleanup
os.remove("test_catalog.mlite")
os.remove("test_catalog.wal")
