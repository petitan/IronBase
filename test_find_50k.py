#!/usr/bin/env python3
"""Test find() with 50K documents"""

from ironbase import IronBase
import os
import time

# Clean up
for f in ["test_50k.mlite", "test_50k.wal"]:
    if os.path.exists(f):
        os.remove(f)

print("=" * 60)
print("Inserting 50K documents...")
db = IronBase("test_50k.mlite")
products = db.collection("products")

# Insert 50K documents in batches
batch_size = 5000
total = 50000

for i in range(0, total, batch_size):
    docs = [
        {"name": f"Product {j}", "category": "Electronics" if j % 7 == 0 else "Books", "price": j}
        for j in range(i, min(i + batch_size, total))
    ]
    products.insert_many(docs)
    if (i + batch_size) % 10000 == 0:
        print(f"  Progress: {i + batch_size}/{total}")

print(f"✓ Inserted {total:,} documents")
file_size = os.path.getsize("test_50k.mlite")
print(f"✓ File size: {file_size / 1024 / 1024:.2f} MB")

# Test count
print("\n" + "=" * 60)
print("Testing count...")
count = products.count_documents({})
print(f"✓ Count: {count:,}")

# Test find
print("\n" + "=" * 60)
print("Testing find()...")
try:
    start = time.time()
    electronics = list(products.find({"category": "Electronics"}))
    elapsed = time.time() - start
    print(f"✓ Found {len(electronics):,} electronics in {elapsed:.2f}s")

    # Print first 3
    for doc in electronics[:3]:
        print(f"  - {doc.get('name')}: ${doc.get('price')}")
except Exception as e:
    print(f"❌ Error: {e}")
    import traceback
    traceback.print_exc()

# Test find_one
print("\n" + "=" * 60)
print("Testing find_one()...")
try:
    doc = products.find_one({})
    print(f"✓ Found: {doc.get('name')}")
except Exception as e:
    print(f"❌ Error: {e}")
    import traceback
    traceback.print_exc()

# Cleanup
db.close()
os.remove("test_50k.mlite")
os.remove("test_50k.wal")
print("\n✓ Test completed")
