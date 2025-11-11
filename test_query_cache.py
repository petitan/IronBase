#!/usr/bin/env python3
"""Test query cache functionality"""
import time
import ironbase

# Create test database
db = ironbase.IronBase("test_cache.mlite")
coll = db.collection("test")

# Insert test data
print("Inserting 10,000 documents...")
for i in range(10000):
    coll.insert_one({"name": f"User{i}", "age": i % 100})

print("\nRunning query 100 times (same query)...")
query = {"age": {"$gte": 25}}

# First 10 queries (should include 1 cache miss + 9 cache hits)
times = []
for i in range(10):
    start = time.perf_counter()
    results = coll.find(query)
    end = time.perf_counter()
    elapsed_ms = (end - start) * 1000
    times.append(elapsed_ms)
    print(f"Query {i+1}: {elapsed_ms:.2f} ms ({len(results)} results)")

print(f"\nFirst query (cache miss): {times[0]:.2f} ms")
print(f"Average of queries 2-10 (cache hits): {sum(times[1:]) / len(times[1:]):.2f} ms")
print(f"Speedup: {times[0] / (sum(times[1:]) / len(times[1:])):.1f}x")

# Clean up
db.close()
import os
os.remove("test_cache.mlite")
