# MongoLite Index System Documentation

## Overview

MongoLite now includes a full-featured B+ tree index system that provides:

- **Automatic indexing** on `_id` field for every collection
- **Custom indexes** on any field (unique and non-unique)
- **Query optimization** with automatic index selection
- **Range queries** optimized with `IndexRangeScan`
- **Query explanation** to see which indexes are used
- **Manual index hints** to override automatic selection

## Performance

With the B+ tree index system:

- **1.26M inserts/sec** - Fast index maintenance during inserts
- **1.39µs per search** - Sub-microsecond index lookups
- **1.4-1.6x speedup** - Real-world query performance improvements
- **O(log n) complexity** - Logarithmic search time (vs O(n) for full scan)

## Python API

### Creating Indexes

```python
import ironbase

db = ironbase.MongoLite("myapp.db")
users = db.collection("users")

# Create a non-unique index on age field
users.create_index("age")

# Create a unique index on email field
users.create_index("email", unique=True)

# List all indexes
indexes = users.list_indexes()
print(indexes)  # ['users_id', 'users_age', 'users_email']
```

### Querying with Indexes

MongoLite automatically selects the best index for your queries:

```python
# Equality query - uses IndexScan
results = users.find({"age": 25})

# Range query - uses IndexRangeScan
results = users.find({
    "age": {
        "$gte": 18,
        "$lt": 65
    }
})

# Query on non-indexed field - uses CollectionScan
results = users.find({"city": "San Francisco"})
```

### Query Explanation

See which indexes are being used:

```python
# Explain a query without executing it
plan = users.explain({"age": 25})

print(plan["queryPlan"])      # "IndexScan"
print(plan["indexUsed"])      # "users_age"
print(plan["stage"])          # "FETCH_WITH_INDEX"
print(plan["estimatedCost"])  # "O(log n)"
```

Example output for different query types:

**IndexScan (equality query):**
```json
{
  "queryPlan": "IndexScan",
  "indexUsed": "users_age",
  "field": "age",
  "stage": "FETCH_WITH_INDEX",
  "indexType": "equality",
  "estimatedCost": "O(log n)"
}
```

**IndexRangeScan (range query):**
```json
{
  "queryPlan": "IndexRangeScan",
  "indexUsed": "users_age",
  "field": "age",
  "stage": "FETCH_WITH_INDEX",
  "indexType": "range",
  "range": {
    "inclusiveStart": true,
    "inclusiveEnd": false
  },
  "estimatedCost": "O(log n + k)"
}
```

**CollectionScan (no index available):**
```json
{
  "queryPlan": "CollectionScan",
  "indexUsed": null,
  "stage": "FULL_SCAN",
  "reason": "No suitable index found for query",
  "estimatedCost": "O(n)"
}
```

### Manual Index Hints

Override automatic index selection:

```python
# Force use of a specific index
results = users.find_with_hint(
    {"age": 25},
    "users_age"  # Index name to use
)
```

This is useful when:
- You know better than the query planner which index to use
- Testing different index strategies
- Debugging query performance issues

### Unique Constraints

Unique indexes prevent duplicate values:

```python
# Create unique index
users.create_index("email", unique=True)

# First insert succeeds
users.insert_one({"name": "Alice", "email": "alice@example.com"})

# Second insert with same email fails
try:
    users.insert_one({"name": "Bob", "email": "alice@example.com"})
except Exception as e:
    print(e)  # Index error: Duplicate key
```

### Dropping Indexes

Remove indexes you no longer need:

```python
# Drop a specific index
users.drop_index("users_age")

# List remaining indexes
print(users.list_indexes())  # ['users_id', 'users_email']
```

**Note:** The automatic `_id` index cannot be dropped.

## Supported Query Operators

The index system optimizes these MongoDB-style operators:

### Equality
```python
{"age": 25}                    # Exact match → IndexScan
```

### Range Operators
```python
{"age": {"$gt": 18}}           # Greater than
{"age": {"$gte": 18}}          # Greater than or equal
{"age": {"$lt": 65}}           # Less than
{"age": {"$lte": 65}}          # Less than or equal
{"age": {"$gte": 18, "$lt": 65}}  # Range → IndexRangeScan
```

### Membership
```python
{"status": {"$in": ["active", "pending"]}}  # Multiple values
```

## Best Practices

### 1. Index Selective Fields

Create indexes on fields that:
- Are frequently queried
- Have high cardinality (many unique values)
- Are used in range queries

```python
# Good candidates for indexes
users.create_index("email", unique=True)  # High cardinality, unique
users.create_index("age")                  # Frequently filtered
users.create_index("created_at")           # Often used in ranges

# Poor candidates
users.create_index("is_active")  # Low cardinality (only true/false)
```

### 2. Use explain() for Optimization

Before adding an index, check if it's being used:

```python
# Check current query plan
plan = users.explain({"age": {"$gte": 18}})
print(plan["queryPlan"])

# If it's CollectionScan, consider adding index
if plan["queryPlan"] == "CollectionScan":
    users.create_index("age")
```

### 3. Unique Constraints for Data Integrity

Use unique indexes to enforce business rules:

```python
# Prevent duplicate emails
users.create_index("email", unique=True)

# Prevent duplicate usernames
users.create_index("username", unique=True)
```

### 4. Monitor Performance

Compare query performance with and without indexes:

```python
import time

# Measure query time
start = time.time()
results = users.find({"age": 25})
query_time = time.time() - start

print(f"Query took {query_time:.4f}s, found {len(results)} results")
```

## Architecture

### B+ Tree Implementation

MongoLite uses a B+ tree with:

- **Order 32** - Each node can hold up to 31 keys
- **In-memory structure** - Fast lookups with no disk I/O during search
- **Automatic balancing** - Maintains O(log n) height through node splits
- **Leaf-level data** - All document IDs stored in leaf nodes

### Query Planner

The query planner analyzes queries and selects execution strategies:

1. **Query Analysis** - Parse query operators (`$gt`, `$gte`, etc.)
2. **Index Selection** - Find best index for the query field
3. **Plan Generation** - Create `IndexScan`, `IndexRangeScan`, or `CollectionScan`
4. **Execution** - Use selected plan to retrieve documents

### Index Maintenance

Indexes are automatically maintained during:

- **insert_one()** - Add key to all relevant indexes
- **update_one() / update_many()** - Update indexes if indexed fields change
- **delete_one() / delete_many()** - Remove keys from indexes

## Limitations

Current limitations (may be addressed in future versions):

1. **Single-field indexes only** - Compound indexes not yet supported
2. **No text search** - Full-text indexes not implemented
3. **In-memory only** - Indexes are rebuilt on database open
4. **No index persistence** - Indexes stored in memory, not on disk
5. **No index rebuild** - Cannot rebuild index from existing documents

## Examples

### Complete Usage Example

```python
import ironbase

# Open database
db = ironbase.MongoLite("ecommerce.db")
products = db.collection("products")

# Create indexes
products.create_index("category")
products.create_index("price")
products.create_index("sku", unique=True)

# Insert products
products.insert_one({
    "name": "Laptop",
    "category": "Electronics",
    "price": 999.99,
    "sku": "LAPTOP-001"
})

# Query with automatic index selection
electronics = products.find({"category": "Electronics"})
print(f"Found {len(electronics)} electronics")

# Range query (uses IndexRangeScan)
affordable = products.find({
    "price": {
        "$gte": 100,
        "$lt": 500
    }
})

# Explain query plan
plan = products.explain({"price": {"$gte": 100, "$lt": 500}})
print(f"Query will use: {plan['queryPlan']}")  # IndexRangeScan
print(f"Index: {plan['indexUsed']}")           # products_price
print(f"Cost: {plan['estimatedCost']}")        # O(log n + k)

# Manual index hint
results = products.find_with_hint(
    {"price": 999.99},
    "products_price"
)

# Cleanup
db.close()
```

## Troubleshooting

### Query Not Using Index

If `explain()` shows `CollectionScan` when you expect `IndexScan`:

1. **Check index exists:** `print(collection.list_indexes())`
2. **Verify field name:** Index must match query field exactly
3. **Check query structure:** Some operators may not use indexes
4. **Use hint():** Force index usage for testing

### Unique Constraint Errors

If you get "Duplicate key" errors:

1. **Check existing data:** Query for duplicates before creating index
2. **Clean up duplicates:** Remove or update duplicate documents
3. **Make index non-unique:** Use `unique=False` if duplicates are acceptable

### Performance Not Improving

If indexes don't speed up queries:

1. **Dataset too small:** Indexes have overhead, benefit shows with larger datasets
2. **Wrong field indexed:** Index should be on filtered field, not result field
3. **Low selectivity:** If index matches most documents, scan may be faster

## Future Enhancements

Planned features for future releases:

- **Compound indexes** - Index on multiple fields
- **Text indexes** - Full-text search capabilities
- **Index persistence** - Save indexes to disk
- **Index statistics** - Track index usage and efficiency
- **Partial indexes** - Index subset of documents matching criteria
- **TTL indexes** - Automatic document expiration

---

For more information, see:
- [MongoLite README](README.md)
- [Architecture Documentation](ARCHITECTURE.md)
- [Query System](IMPLEMENTATION_QUERY_OPTIMIZER.md)
