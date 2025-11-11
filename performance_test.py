#!/usr/bin/env python3
"""
MongoLite Performance Testing
Measures insert, find, update, delete, and query performance
"""
import ironbase
import time
import os
import statistics

def measure_time(func, *args, **kwargs):
    """Measure execution time of a function"""
    start = time.perf_counter()
    result = func(*args, **kwargs)
    end = time.perf_counter()
    return end - start, result

def format_time(seconds):
    """Format time in appropriate unit"""
    if seconds < 0.001:
        return f"{seconds * 1_000_000:.2f} μs"
    elif seconds < 1:
        return f"{seconds * 1_000:.2f} ms"
    else:
        return f"{seconds:.2f} s"

def format_throughput(count, seconds):
    """Format throughput"""
    ops_per_sec = count / seconds if seconds > 0 else 0
    return f"{ops_per_sec:,.0f} ops/sec"

def benchmark_insert(db_path, num_docs):
    """Benchmark insert_one performance"""
    print(f"\n{'='*70}")
    print(f"INSERT Benchmark - {num_docs:,} documents")
    print('='*70)

    if os.path.exists(db_path):
        os.remove(db_path)

    db = ironbase.MongoLite(db_path)
    coll = db.collection("users")

    # Warmup
    for i in range(10):
        coll.insert_one({"name": f"Warmup{i}", "age": i})

    # Actual benchmark
    start = time.perf_counter()
    for i in range(num_docs):
        coll.insert_one({
            "name": f"User{i}",
            "age": 20 + (i % 50),
            "city": ["NYC", "LA", "SF"][i % 3],
            "active": i % 2 == 0
        })
    end = time.perf_counter()

    duration = end - start

    print(f"  Total time: {format_time(duration)}")
    print(f"  Throughput: {format_throughput(num_docs, duration)}")
    print(f"  Avg per insert: {format_time(duration / num_docs)}")

    # Create indexes for benchmarked fields
    print(f"\n  Creating indexes...")
    start_index = time.perf_counter()
    coll.create_index("age")
    coll.create_index("name")
    end_index = time.perf_counter()
    print(f"  Index creation time: {format_time(end_index - start_index)}")

    db.close()
    return duration

def benchmark_find(db_path, num_queries):
    """Benchmark find performance"""
    print(f"\n{'='*70}")
    print(f"FIND Benchmark - {num_queries:,} queries")
    print('='*70)

    db = ironbase.MongoLite(db_path)
    coll = db.collection("users")

    # find() all documents
    start = time.perf_counter()
    results = coll.find({})
    end = time.perf_counter()

    duration1 = end - start
    print(f"  find() all: {format_time(duration1)} ({len(results)} docs)")

    # find() with filter
    start = time.perf_counter()
    for i in range(num_queries):
        results = coll.find({"age": {"$gte": 25}})
    end = time.perf_counter()

    duration2 = end - start
    print(f"  find() filtered ({num_queries} queries): {format_time(duration2)}")
    print(f"  Throughput: {format_throughput(num_queries, duration2)}")
    print(f"  Avg per query: {format_time(duration2 / num_queries)}")

    # find_one()
    times = []
    for i in range(num_queries):
        elapsed, _ = measure_time(coll.find_one, {"name": f"User{i % 1000}"})
        times.append(elapsed)

    print(f"  find_one() avg: {format_time(statistics.mean(times))}")
    print(f"  find_one() median: {format_time(statistics.median(times))}")

    db.close()
    return duration1, duration2

def benchmark_update(db_path, num_updates):
    """Benchmark update performance"""
    print(f"\n{'='*70}")
    print(f"UPDATE Benchmark - {num_updates:,} updates")
    print('='*70)

    db = ironbase.MongoLite(db_path)
    coll = db.collection("users")

    # update_one()
    start = time.perf_counter()
    for i in range(num_updates):
        coll.update_one(
            {"name": f"User{i % 1000}"},
            {"$inc": {"age": 1}}
        )
    end = time.perf_counter()

    duration = end - start
    print(f"  update_one() total: {format_time(duration)}")
    print(f"  Throughput: {format_throughput(num_updates, duration)}")
    print(f"  Avg per update: {format_time(duration / num_updates)}")

    db.close()
    return duration

def benchmark_delete(db_path, num_deletes):
    """Benchmark delete performance"""
    print(f"\n{'='*70}")
    print(f"DELETE Benchmark - {num_deletes:,} deletes")
    print('='*70)

    db = ironbase.MongoLite(db_path)
    coll = db.collection("users")

    # delete_one()
    start = time.perf_counter()
    for i in range(num_deletes):
        coll.delete_one({"name": f"User{i}"})
    end = time.perf_counter()

    duration = end - start
    print(f"  delete_one() total: {format_time(duration)}")
    print(f"  Throughput: {format_throughput(num_deletes, duration)}")
    print(f"  Avg per delete: {format_time(duration / num_deletes)}")

    db.close()
    return duration

def benchmark_count(db_path, num_queries):
    """Benchmark count performance"""
    print(f"\n{'='*70}")
    print(f"COUNT Benchmark - {num_queries:,} queries")
    print('='*70)

    db = ironbase.MongoLite(db_path)
    coll = db.collection("users")

    # count_documents()
    start = time.perf_counter()
    for i in range(num_queries):
        count = coll.count_documents({"age": {"$gte": 25}})
    end = time.perf_counter()

    duration = end - start
    print(f"  count_documents() total: {format_time(duration)}")
    print(f"  Throughput: {format_throughput(num_queries, duration)}")
    print(f"  Avg per count: {format_time(duration / num_queries)}")

    db.close()
    return duration

def benchmark_compaction(db_path):
    """Benchmark compaction performance"""
    print(f"\n{'='*70}")
    print(f"COMPACTION Benchmark")
    print('='*70)

    db = ironbase.MongoLite(db_path)

    size_before = os.path.getsize(db_path)

    start = time.perf_counter()
    stats = db.compact()
    end = time.perf_counter()

    duration = end - start
    print(f"  Compaction time: {format_time(duration)}")
    print(f"  Size before: {stats['size_before']:,} bytes")
    print(f"  Size after: {stats['size_after']:,} bytes")
    print(f"  Space saved: {stats['space_saved']:,} bytes ({stats['space_saved'] / stats['size_before'] * 100:.1f}%)")
    print(f"  Documents kept: {stats['documents_kept']}")
    print(f"  Tombstones removed: {stats['tombstones_removed']}")

    db.close()
    return duration

def main():
    print("=" * 70)
    print("MongoLite Performance Benchmark Suite")
    print("=" * 70)

    db_path = "perf_test.mlite"

    # Configuration
    num_docs = 10_000
    num_queries = 1_000
    num_updates = 1_000
    num_deletes = 100

    # Run benchmarks
    insert_time = benchmark_insert(db_path, num_docs)
    find_time = benchmark_find(db_path, num_queries)
    update_time = benchmark_update(db_path, num_updates)
    delete_time = benchmark_delete(db_path, num_deletes)
    count_time = benchmark_count(db_path, num_queries)
    compact_time = benchmark_compaction(db_path)

    # Summary
    print(f"\n{'='*70}")
    print("SUMMARY")
    print('='*70)
    print(f"  INSERT: {num_docs:,} docs in {format_time(insert_time)} = {format_throughput(num_docs, insert_time)}")
    print(f"  FIND:   {num_queries:,} queries in {format_time(find_time[1])} = {format_throughput(num_queries, find_time[1])}")
    print(f"  UPDATE: {num_updates:,} updates in {format_time(update_time)} = {format_throughput(num_updates, update_time)}")
    print(f"  DELETE: {num_deletes:,} deletes in {format_time(delete_time)} = {format_throughput(num_deletes, delete_time)}")
    print(f"  COUNT:  {num_queries:,} counts in {format_time(count_time)} = {format_throughput(num_queries, count_time)}")
    print(f"  COMPACT: {format_time(compact_time)}")

    # Database info
    final_size = os.path.getsize(db_path)
    print(f"\n  Final database size: {final_size:,} bytes ({final_size / 1024 / 1024:.2f} MB)")

    # Clean up
    if os.path.exists(db_path):
        os.remove(db_path)

    print(f"\n{'='*70}")
    print("✅ Performance benchmark completed!")
    print('='*70)

if __name__ == "__main__":
    main()
