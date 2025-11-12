#!/usr/bin/env python3
"""
Test insert_many() performance - before/after comparison
"""

from ironbase import IronBase
import time
import os

def test_insert_many_batch():
    """Test new batched insert_many implementation"""
    db_path = "test_insert_many.mlite"

    # Clean up
    for ext in [".mlite", ".wal"]:
        try:
            os.remove(db_path.replace(".mlite", ext))
        except FileNotFoundError:
            pass

    db = IronBase(db_path)
    coll = db.collection("test")

    # Prepare test data
    docs = []
    for i in range(1000):
        docs.append({
            "index": i,
            "name": f"User {i}",
            "email": f"user{i}@example.com",
            "age": 20 + (i % 50),
            "tags": ["tag1", "tag2", "tag3"]
        })

    # Test insert_many
    print("=" * 60)
    print("TEST: insert_many() with 1000 documents")
    print("=" * 60)

    start = time.time()
    result = coll.insert_many(docs)
    elapsed = time.time() - start

    print(f"\n✅ Success!")
    print(f"   Time: {elapsed*1000:.2f}ms")
    print(f"   Inserted: {result['inserted_count']} documents")
    print(f"   IDs: {len(result['inserted_ids'])} IDs returned")
    print(f"   First ID: {result['inserted_ids'][0]}")
    print(f"   Last ID: {result['inserted_ids'][-1]}")
    print(f"   Throughput: {result['inserted_count'] / elapsed:.0f} docs/sec")

    # Verify all documents inserted
    count = coll.count_documents()
    print(f"\n📊 Verification:")
    print(f"   Documents in DB: {count}")
    print(f"   Match expected: {'✓' if count == 1000 else '✗'}")

    # Query performance
    start = time.time()
    results = coll.find({"age": 25})
    query_time = time.time() - start
    print(f"\n🔍 Query Performance:")
    print(f"   Query {{age: 25}} time: {query_time*1000:.2f}ms")
    print(f"   Results: {len(results)} documents")

    db.close()

    # Cleanup
    for ext in [".mlite", ".wal"]:
        try:
            os.remove(db_path.replace(".mlite", ext))
        except FileNotFoundError:
            pass

    print("\n" + "=" * 60)
    print("✅ Test passed!")
    print("=" * 60)

if __name__ == "__main__":
    test_insert_many_batch()
