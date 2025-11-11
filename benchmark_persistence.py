#!/usr/bin/env python3
"""
Performance benchmark for persistence with RESERVED SPACE fix
Tests with 1000, 5000, and 10000 documents
"""
import ironbase
import time
import random
import string

def generate_document(i):
    """Generate a random document"""
    return {
        "user_id": i,
        "username": f"user_{i}",
        "email": f"user{i}@example.com",
        "age": random.randint(18, 80),
        "city": random.choice(["New York", "London", "Tokyo", "Paris", "Berlin"]),
        "tags": [random.choice(string.ascii_lowercase) for _ in range(3)],
        "score": random.randint(0, 1000),
        "active": random.choice([True, False])
    }

def benchmark(num_docs):
    """Benchmark insert and reopen performance"""
    print(f"\n{'='*60}")
    print(f"BENCHMARK: {num_docs} documents")
    print(f"{'='*60}")

    db_file = f"bench_{num_docs}.mlite"

    # === INSERT PERFORMANCE ===
    print(f"\n📝 Inserting {num_docs} documents...")
    db = ironbase.IronBase(db_file)
    collection = db.collection("users")

    start = time.time()
    for i in range(num_docs):
        doc = generate_document(i)
        collection.insert_one(doc)
        if (i + 1) % 1000 == 0:
            print(f"   Inserted {i+1}/{num_docs}...")
    insert_time = time.time() - start

    print(f"\n✅ Insert complete:")
    print(f"   Time: {insert_time:.2f}s")
    print(f"   Rate: {num_docs/insert_time:.0f} docs/sec")

    # === CLOSE & FLUSH ===
    print(f"\n💾 Closing database (flush metadata)...")
    start = time.time()
    db.close()
    close_time = time.time() - start
    print(f"   Time: {close_time:.3f}s")

    # === REOPEN PERFORMANCE ===
    print(f"\n📂 Reopening database...")
    start = time.time()
    db2 = ironbase.IronBase(db_file)
    collection2 = db2.collection("users")
    reopen_time = time.time() - start
    print(f"   Time: {reopen_time:.3f}s")

    # === VERIFY DATA ===
    print(f"\n🔍 Verifying data integrity...")
    start = time.time()
    count = collection2.count_documents({})
    verify_time = time.time() - start

    if count == num_docs:
        print(f"   ✅ Count: {count}/{num_docs} (CORRECT)")
    else:
        print(f"   ❌ Count: {count}/{num_docs} (WRONG!)")
        return False

    print(f"   Count time: {verify_time:.3f}s")

    # === QUERY PERFORMANCE ===
    print(f"\n🔍 Query performance:")

    # Simple query
    start = time.time()
    results = collection2.find({"city": "Tokyo"})
    query_time = time.time() - start
    print(f"   Find by city: {len(results)} docs in {query_time:.3f}s")

    # Indexed query (_id)
    start = time.time()
    result = collection2.find_one({"_id": num_docs // 2})
    indexed_time = time.time() - start
    print(f"   Find by _id: {indexed_time*1000:.2f}ms")

    # Aggregation
    start = time.time()
    cities = collection2.distinct("city")
    distinct_time = time.time() - start
    print(f"   Distinct cities: {len(cities)} in {distinct_time:.3f}s")

    db2.close()

    # === SUMMARY ===
    print(f"\n📊 Summary:")
    print(f"   Total documents: {num_docs}")
    print(f"   Insert rate: {num_docs/insert_time:.0f} docs/sec")
    print(f"   Close time: {close_time:.3f}s")
    print(f"   Reopen time: {reopen_time:.3f}s")
    print(f"   Data integrity: ✅ PASSED")

    return True

if __name__ == "__main__":
    print("🚀 IronBase Persistence Performance Benchmark")
    print("Testing RESERVED SPACE implementation")

    # Clean up old files
    import os
    for f in ["bench_1000.mlite", "bench_5000.mlite", "bench_10000.mlite"]:
        if os.path.exists(f):
            os.remove(f)

    # Run benchmarks
    sizes = [1000, 5000, 10000]

    for size in sizes:
        try:
            if not benchmark(size):
                print(f"\n❌ Benchmark FAILED for {size} documents!")
                break
        except Exception as e:
            print(f"\n❌ Error: {e}")
            import traceback
            traceback.print_exc()
            break
    else:
        print(f"\n{'='*60}")
        print("✅ ALL BENCHMARKS PASSED!")
        print(f"{'='*60}")
