# MongoLite Performance Guide

This document describes performance testing and optimization for MongoLite.

## Performance Testing

### Python Performance Test

Run the comprehensive performance benchmark:

```bash
python performance_test.py
```

This benchmarks:
- **INSERT**: 10,000 document inserts
- **FIND**: 1,000 queries (all documents + filtered)
- **UPDATE**: 1,000 updates with `$inc` operator
- **DELETE**: 100 deletes with tombstone pattern
- **COUNT**: 1,000 count queries with filters
- **COMPACT**: Garbage collection performance

### Rust Benchmarks (Criterion)

**Note**: Requires Rust 1.80+ for criterion. Currently disabled due to Rust 1.75.0.

```bash
cargo bench --bench benchmarks
```

Benchmarks include:
- Document creation/serialization/deserialization
- Storage write with varying sizes (100B - 100KB)
- CRUD operations (insert, find, update, delete)
- Complex queries ($and, $or, range queries)

## Performance Characteristics

### Storage Architecture
- **RESERVED SPACE**: 256KB reserved for metadata
- **Append-only writes**: Documents appended to end of file
- **Memory-mapped I/O**: For files < 1GB
- **4KB pages**: Index nodes stored in 4KB pages

### Indexing
- **B+ tree**: Balanced tree for fast lookups
- **Range queries**: O(log n) seek + O(k) scan
- **Index persistence**: JSON serialization to 4KB pages

### Compaction
- **Garbage collection**: Removes tombstones and old versions
- **Atomic replacement**: Creates new file, then atomic rename
- **RESERVED SPACE preserved**: Maintains consistent file layout

## Optimization Tips

###  1. Use Indexes for Frequent Queries

```python
# Create index on frequently queried field
collection.create_index("email", unique=True)

# Now this query uses the index
results = collection.find({"email": "user@example.com"})
```

### 2. Batch Inserts

```python
# Instead of multiple insert_one calls:
for doc in documents:
    collection.insert_one(doc)  # Slower

# Use insert_many:
collection.insert_many(documents)  # Faster
```

### 3. Use Projections

```python
# Fetch only needed fields
results = collection.find(
    {"age": {"$gt": 25}},
    projection={"name": 1, "email": 1}
)
```

### 4. Regular Compaction

```python
# Run compaction after bulk deletes
collection.delete_many({"active": False})
db.compact()  # Reclaim space
```

### 5. Limit Result Sets

```python
# Use limit for large result sets
results = collection.find(
    {"category": "books"},
    limit=100
)
```

## Expected Performance

### Typical Throughput (10K documents)

- **INSERT**: 1,000-5,000 ops/sec
- **FIND (all)**: 100-500 ops/sec
- **FIND (filtered)**: 200-1,000 ops/sec
- **UPDATE**: 500-2,000 ops/sec
- **DELETE**: 500-2,000 ops/sec
- **COUNT**: 500-2,000 ops/sec

*Note: Actual performance depends on document size, query complexity, and hardware.*

### Scalability

- **Documents**: Tested up to 1M documents
- **File size**: Efficient up to 1GB (memory-mapped I/O)
- **Query performance**: O(n) full scan, O(log n) with indexes
- **Memory usage**: ~10MB + document cache

## Profiling

### Enable Debug Logging

Set Rust log level:

```bash
export RUST_LOG=ironbase_core=debug
python your_script.py
```

### Measure Operation Time

```python
import time

start = time.perf_counter()
collection.insert_many(documents)
end = time.perf_counter()

print(f"Insert time: {(end - start) * 1000:.2f} ms")
```

### Database Statistics

```python
stats = db.stats()
print(f"Collections: {stats['collection_count']}")
print(f"File size: {stats['file_size']} bytes")
```

## Performance Considerations

### Strengths
✅ Fast single-document operations
✅ Efficient range queries with indexes
✅ Low memory footprint
✅ Memory-mapped I/O for large files
✅ Atomic transactions (ACD)

### Limitations
⚠️ Full table scans for unindexed queries
⚠️ No distributed queries
⚠️ RESERVED SPACE limits compaction gains
⚠️ Large result sets loaded into memory
⚠️ No query planner/optimizer

## Future Optimizations

- [ ] Query planner for index selection
- [ ] Cursor-based pagination
- [ ] Bloom filters for existence checks
- [ ] WAL (Write-Ahead Log) for crash recovery
- [ ] Connection pooling for concurrent access
- [ ] Aggregation pipeline optimization

## Benchmarking Your Workload

Create a custom benchmark for your use case:

```python
import time
import ironbase

db = ironbase.MongoLite("my_benchmark.mlite")
coll = db.collection("test")

# Your workload here
start = time.perf_counter()
for i in range(10000):
    coll.insert_one({"data": f"test{i}"})
duration = time.perf_counter() - start

print(f"Throughput: {10000 / duration:.0f} ops/sec")
db.close()
```

## Performance FAQs

**Q: Why is find() slow on large collections?**
A: Without an index, find() performs a full table scan (O(n)). Create an index on frequently queried fields.

**Q: Does compaction improve read performance?**
A: Yes, by reducing file size and removing tombstones, compaction can improve sequential scan performance.

**Q: How do I optimize bulk operations?**
A: Use `insert_many()`, `update_many()`, `delete_many()` instead of loops with single-document operations.

**Q: What's the largest database size MongoLite can handle?**
A: Tested up to 1GB files (memory-mapped I/O limit). Larger files work but with standard file I/O (slower).
