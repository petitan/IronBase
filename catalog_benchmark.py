#!/usr/bin/env python3
"""
Document Catalog Optimization Benchmark
Tests the performance improvement from HashMap<DocumentId, u64>
"""

import ironbase
import time
import os

def benchmark_operations(db_path, num_docs=10000):
    """Benchmark insert, find, update, delete operations"""

    # Clean up existing DB
    if os.path.exists(db_path):
        os.remove(db_path)

    db = ironbase.IronBase(db_path)
    coll = db.collection("benchmark")

    print(f"🔥 Document Catalog Optimization Benchmark")
    print(f"📊 Testing with {num_docs:,} documents\n")

    # === INSERT BENCHMARK ===
    print("1️⃣  INSERT Performance:")
    docs = [{"_id": i, "name": f"User {i}", "age": 20 + (i % 50)} for i in range(num_docs)]

    start = time.time()
    for doc in docs:
        coll.insert_one(doc)
    insert_time = time.time() - start

    print(f"   ✅ Inserted {num_docs:,} docs in {insert_time:.3f}s")
    print(f"   ⚡ {num_docs/insert_time:.0f} inserts/sec\n")

    # === FIND BY ID BENCHMARK ===
    print("2️⃣  FIND BY _ID (O(1) catalog lookup):")

    start = time.time()
    found_count = 0
    for i in range(0, num_docs, 100):  # Sample every 100th doc
        result = coll.find_one({"_id": i})
        if result is not None:
            found_count += 1
    find_time = time.time() - start

    queries = num_docs // 100
    print(f"   ✅ Found {found_count}/{queries} docs by _id in {find_time:.3f}s")
    if found_count > 0:
        print(f"   ⚡ {found_count/find_time:.0f} lookups/sec\n")
    else:
        print(f"   ⚠️  WARNING: No documents found - persistence issue?\n")

    # === UPDATE BENCHMARK ===
    print("3️⃣  UPDATE BY _ID:")

    start = time.time()
    for i in range(0, min(1000, num_docs)):
        coll.update_one({"_id": i}, {"$set": {"updated": True}})
    update_time = time.time() - start

    updates = min(1000, num_docs)
    print(f"   ✅ Updated {updates:,} docs in {update_time:.3f}s")
    print(f"   ⚡ {updates/update_time:.0f} updates/sec\n")

    # === DELETE BENCHMARK ===
    print("4️⃣  DELETE BY _ID:")

    start = time.time()
    for i in range(0, min(1000, num_docs)):
        coll.delete_one({"_id": i})
    delete_time = time.time() - start

    deletes = min(1000, num_docs)
    print(f"   ✅ Deleted {deletes:,} docs in {delete_time:.3f}s")
    print(f"   ⚡ {deletes/delete_time:.0f} deletes/sec\n")

    # === SUMMARY ===
    print("=" * 60)
    print("📈 PERFORMANCE SUMMARY:")
    print(f"   Insert:  {num_docs/insert_time:.0f} ops/sec")
    print(f"   Find:    {queries/find_time:.0f} ops/sec")
    print(f"   Update:  {updates/update_time:.0f} ops/sec")
    print(f"   Delete:  {deletes/delete_time:.0f} ops/sec")
    print("=" * 60)

    # Clean up
    os.remove(db_path)

if __name__ == "__main__":
    benchmark_operations("catalog_bench.mlite", num_docs=10000)
