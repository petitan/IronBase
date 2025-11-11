#!/usr/bin/env python3
"""
Memory usage benchmark for IronBase
Tests memory consumption under various scenarios
"""

import sys
import os
import psutil
import time
import gc
from pathlib import Path

# Add parent directory to path for imports
sys.path.insert(0, str(Path(__file__).parent))

try:
    import ironbase
except ImportError:
    print("Error: ironbase module not found. Run 'maturin develop' first.")
    sys.exit(1)


def get_memory_usage():
    """Get current memory usage in MB"""
    process = psutil.Process()
    mem_info = process.memory_info()
    return mem_info.rss / 1024 / 1024  # Convert to MB


def measure_memory(func, *args, **kwargs):
    """Measure memory usage of a function"""
    gc.collect()
    mem_before = get_memory_usage()

    result = func(*args, **kwargs)

    gc.collect()
    mem_after = get_memory_usage()

    mem_used = mem_after - mem_before
    return result, mem_used, mem_before, mem_after


def test_database_creation():
    """Test memory for database creation"""
    print("\n" + "="*60)
    print("TEST 1: Database Creation")
    print("="*60)

    def create_db():
        db_path = "test_memory_create.mlite"
        if os.path.exists(db_path):
            os.remove(db_path)
        if os.path.exists(db_path + ".wal"):
            os.remove(db_path + ".wal")

        db = ironbase.IronBase(db_path)
        return db

    db, mem_used, mem_before, mem_after = measure_memory(create_db)

    print(f"Memory before: {mem_before:.2f} MB")
    print(f"Memory after:  {mem_after:.2f} MB")
    print(f"Memory used:   {mem_used:.2f} MB")

    # Cleanup
    db_path = "test_memory_create.mlite"
    if os.path.exists(db_path):
        os.remove(db_path)
    if os.path.exists(db_path + ".wal"):
        os.remove(db_path + ".wal")

    return mem_used


def test_bulk_insert(num_documents=10000):
    """Test memory for bulk inserts"""
    print("\n" + "="*60)
    print(f"TEST 2: Bulk Insert ({num_documents:,} documents)")
    print("="*60)

    db_path = "test_memory_insert.mlite"
    if os.path.exists(db_path):
        os.remove(db_path)
    if os.path.exists(db_path + ".wal"):
        os.remove(db_path + ".wal")

    db = ironbase.IronBase(db_path)
    collection = db.collection("users")

    def bulk_insert():
        for i in range(num_documents):
            collection.insert_one({
                "name": f"User {i}",
                "email": f"user{i}@example.com",
                "age": 20 + (i % 50),
                "score": i * 1.5,
                "active": i % 2 == 0,
                "tags": [f"tag{i % 10}", f"category{i % 5}"],
                "metadata": {
                    "created": f"2024-01-{(i % 28) + 1:02d}",
                    "region": ["US", "EU", "ASIA"][i % 3]
                }
            })

    _, mem_used, mem_before, mem_after = measure_memory(bulk_insert)

    print(f"Memory before: {mem_before:.2f} MB")
    print(f"Memory after:  {mem_after:.2f} MB")
    print(f"Memory used:   {mem_used:.2f} MB")
    print(f"Per document:  {(mem_used * 1024) / num_documents:.2f} KB")

    # Cleanup
    if os.path.exists(db_path):
        os.remove(db_path)
    if os.path.exists(db_path + ".wal"):
        os.remove(db_path + ".wal")

    return mem_used


def test_index_creation():
    """Test memory for index creation"""
    print("\n" + "="*60)
    print("TEST 3: Index Creation (1000 documents)")
    print("="*60)

    db_path = "test_memory_index.mlite"
    if os.path.exists(db_path):
        os.remove(db_path)
    if os.path.exists(db_path + ".wal"):
        os.remove(db_path + ".wal")

    db = ironbase.IronBase(db_path)
    collection = db.collection("users")

    # Insert documents first
    for i in range(1000):
        collection.insert_one({
            "name": f"User {i}",
            "age": 20 + (i % 50),
            "score": i * 1.5
        })

    gc.collect()
    mem_before = get_memory_usage()

    # Create indexes
    collection.create_index("age")
    collection.create_index("score")
    collection.create_index("name")

    gc.collect()
    mem_after = get_memory_usage()
    mem_used = mem_after - mem_before

    print(f"Memory before: {mem_before:.2f} MB")
    print(f"Memory after:  {mem_after:.2f} MB")
    print(f"Memory used:   {mem_used:.2f} MB")
    print(f"Per index:     {mem_used / 3:.2f} MB")

    # Cleanup
    if os.path.exists(db_path):
        os.remove(db_path)
    if os.path.exists(db_path + ".wal"):
        os.remove(db_path + ".wal")

    return mem_used


def test_query_operations():
    """Test memory for query operations"""
    print("\n" + "="*60)
    print("TEST 4: Query Operations (5000 documents)")
    print("="*60)

    db_path = "test_memory_query.mlite"
    if os.path.exists(db_path):
        os.remove(db_path)
    if os.path.exists(db_path + ".wal"):
        os.remove(db_path + ".wal")

    db = ironbase.IronBase(db_path)
    collection = db.collection("users")

    # Insert test data
    for i in range(5000):
        collection.insert_one({
            "name": f"User {i}",
            "age": 20 + (i % 50),
            "score": i * 1.5
        })

    collection.create_index("age")

    gc.collect()
    mem_before = get_memory_usage()

    # Perform various queries
    results1 = collection.find({"age": {"$gt": 30}})
    results2 = collection.find({"age": {"$gte": 25, "$lte": 35}})
    results3 = collection.find({"score": {"$gt": 5000}})

    # Force evaluation
    list(results1)
    list(results2)
    list(results3)

    gc.collect()
    mem_after = get_memory_usage()
    mem_used = mem_after - mem_before

    print(f"Memory before: {mem_before:.2f} MB")
    print(f"Memory after:  {mem_after:.2f} MB")
    print(f"Memory used:   {mem_used:.2f} MB")

    # Cleanup
    if os.path.exists(db_path):
        os.remove(db_path)
    if os.path.exists(db_path + ".wal"):
        os.remove(db_path + ".wal")

    return mem_used


def test_concurrent_collections():
    """Test memory with multiple collections"""
    print("\n" + "="*60)
    print("TEST 5: Multiple Collections (10 collections, 100 docs each)")
    print("="*60)

    db_path = "test_memory_multi.mlite"
    if os.path.exists(db_path):
        os.remove(db_path)
    if os.path.exists(db_path + ".wal"):
        os.remove(db_path + ".wal")

    db = ironbase.IronBase(db_path)

    gc.collect()
    mem_before = get_memory_usage()

    # Create multiple collections with data
    for coll_num in range(10):
        collection = db.collection(f"collection_{coll_num}")
        for i in range(100):
            collection.insert_one({
                "id": i,
                "data": f"Data for collection {coll_num}, item {i}",
                "value": i * coll_num
            })

    gc.collect()
    mem_after = get_memory_usage()
    mem_used = mem_after - mem_before

    print(f"Memory before: {mem_before:.2f} MB")
    print(f"Memory after:  {mem_after:.2f} MB")
    print(f"Memory used:   {mem_used:.2f} MB")
    print(f"Per collection: {mem_used / 10:.2f} MB")

    # Cleanup
    if os.path.exists(db_path):
        os.remove(db_path)
    if os.path.exists(db_path + ".wal"):
        os.remove(db_path + ".wal")

    return mem_used


def test_large_documents():
    """Test memory usage with large documents"""
    print("\n" + "="*60)
    print("TEST 6: Large Documents (100 docs x 10KB each)")
    print("="*60)

    db_path = "test_memory_large.mlite"
    if os.path.exists(db_path):
        os.remove(db_path)
    if os.path.exists(db_path + ".wal"):
        os.remove(db_path + ".wal")

    db = ironbase.IronBase(db_path)
    collection = db.collection("documents")

    # Create large documents (10KB each)
    large_string = "x" * 10000  # 10KB

    gc.collect()
    mem_before = get_memory_usage()

    # Insert large documents
    for i in range(100):
        collection.insert_one({
            "id": i,
            "data": large_string,
            "metadata": {
                "size": len(large_string),
                "index": i,
                "timestamp": f"2024-01-{(i % 28) + 1:02d}"
            }
        })

    gc.collect()
    mem_after = get_memory_usage()
    mem_used = mem_after - mem_before

    print(f"Memory before: {mem_before:.2f} MB")
    print(f"Memory after:  {mem_after:.2f} MB")
    print(f"Memory used:   {mem_used:.2f} MB")
    print(f"Per document:  {mem_used / 100:.2f} MB")
    print(f"Total data size: {(100 * 10) / 1024:.2f} MB")
    print(f"Overhead:      {((mem_used / ((100 * 10) / 1024)) - 1) * 100:.1f}%")

    # Cleanup
    if os.path.exists(db_path):
        os.remove(db_path)
    if os.path.exists(db_path + ".wal"):
        os.remove(db_path + ".wal")

    return mem_used


def main():
    print("="*60)
    print("IronBase Memory Usage Benchmark")
    print("="*60)
    print(f"Python version: {sys.version}")
    print(f"IronBase version: {ironbase.__version__ if hasattr(ironbase, '__version__') else 'unknown'}")

    # Get system info
    process = psutil.Process()
    print(f"Initial memory: {get_memory_usage():.2f} MB")
    print(f"CPU count: {psutil.cpu_count()}")

    results = {}

    try:
        results['db_creation'] = test_database_creation()
        results['bulk_insert'] = test_bulk_insert(10000)
        results['index_creation'] = test_index_creation()
        results['query_ops'] = test_query_operations()
        results['multi_collections'] = test_concurrent_collections()
        results['large_docs'] = test_large_documents()

        # Summary
        print("\n" + "="*60)
        print("SUMMARY")
        print("="*60)
        print(f"Database Creation:     {results['db_creation']:>8.2f} MB")
        print(f"Bulk Insert (10K):     {results['bulk_insert']:>8.2f} MB")
        print(f"Index Creation (3):    {results['index_creation']:>8.2f} MB")
        print(f"Query Operations:      {results['query_ops']:>8.2f} MB")
        print(f"Multi Collections:     {results['multi_collections']:>8.2f} MB")
        print(f"Large Documents:       {results['large_docs']:>8.2f} MB")
        print(f"{'':>23}{'─'*12}")
        print(f"Total Memory Impact:   {sum(results.values()):>8.2f} MB")

        print("\n" + "="*60)
        print("✅ Memory benchmark completed successfully!")
        print("="*60)

    except Exception as e:
        print(f"\n❌ Error during benchmark: {e}")
        import traceback
        traceback.print_exc()
        sys.exit(1)


if __name__ == "__main__":
    main()
