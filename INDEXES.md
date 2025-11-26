# Indexing Guide

IronBase uses B+ tree indexes for fast lookups and range queries.

## Table of Contents

- [Overview](#overview)
- [Creating Indexes](#creating-indexes)
- [Compound Indexes](#compound-indexes)
- [Query Planning](#query-planning)
- [Index Selection](#index-selection)
- [Performance](#performance)
- [Best Practices](#best-practices)

## Overview

Indexes dramatically improve query performance:

| Operation | Without Index | With Index |
|-----------|---------------|------------|
| Equality lookup | O(n) | O(log n) |
| Range query | O(n) | O(log n + k) |
| Sort | O(n log n) | O(n) or O(k) |

IronBase automatically creates an index on `_id` for every collection.

## Creating Indexes

### Single Field Index

```python
# Non-unique index
users.create_index("age")

# Unique index (enforces uniqueness)
users.create_index("email", unique=True)
```

### Index on Nested Fields

```python
# Index nested field using dot notation
users.create_index("address.city")
users.create_index("profile.settings.theme")
```

### Listing Indexes

```python
indexes = users.list_indexes()
print(indexes)
# ['users_id', 'users_age', 'users_email', 'users_address.city']
```

### Dropping Indexes

```python
users.drop_index("users_age")
```

**Note:** Cannot drop the `_id` index.

## Compound Indexes

Compound indexes support queries on multiple fields.

### Creating Compound Indexes

```python
# Index on (country, city)
users.create_compound_index(["country", "city"])

# Unique compound index
orders.create_compound_index(["user_id", "order_id"], unique=True)
```

### Prefix Matching

Compound indexes support queries on the **prefix** of fields:

```python
# Index: (country, city, zip)
users.create_compound_index(["country", "city", "zip"])

# These queries CAN use the index:
users.find({"country": "US"})                          # prefix (1 field)
users.find({"country": "US", "city": "NYC"})           # prefix (2 fields)
users.find({"country": "US", "city": "NYC", "zip": "10001"})  # full match

# These queries CANNOT use the index efficiently:
users.find({"city": "NYC"})              # not a prefix
users.find({"zip": "10001"})             # not a prefix
users.find({"city": "NYC", "zip": "10001"})  # not a prefix
```

### When to Use Compound Indexes

```python
# Scenario: Frequently query by category + price range
products.create_compound_index(["category", "price"])

# Now this is fast:
products.find({"category": "Electronics", "price": {"$lt": 500}})
```

## Query Planning

### explain()

View the query execution plan:

```python
plan = users.explain({"age": {"$gte": 25}})
print(plan)
```

Output:

```json
{
    "queryPlan": "IndexRangeScan",
    "indexUsed": "users_age",
    "estimatedCost": "O(log n + k)",
    "query": {"age": {"$gte": 25}}
}
```

### Query Plan Types

| Plan | Description | When Used |
|------|-------------|-----------|
| `IndexScan` | Exact match on index | `{"field": value}` |
| `IndexRangeScan` | Range on index | `{"field": {"$gt": x}}` |
| `CollectionScan` | Full scan | No suitable index |

### Example Plans

```python
# Equality - IndexScan
users.explain({"email": "alice@example.com"})
# {"queryPlan": "IndexScan", "indexUsed": "users_email", ...}

# Range - IndexRangeScan
users.explain({"age": {"$gte": 18, "$lt": 65}})
# {"queryPlan": "IndexRangeScan", "indexUsed": "users_age", ...}

# No index - CollectionScan
users.explain({"name": "Alice"})
# {"queryPlan": "CollectionScan", "indexUsed": null, ...}
```

## Index Selection

### Automatic Selection

IronBase automatically selects the best index:

```python
# Creates indexes
users.create_index("age")
users.create_index("city")

# Query uses age index (most selective for range)
users.find({"age": {"$gte": 25}, "city": "NYC"})
```

### Manual Selection (hint)

Force a specific index:

```python
# Use city index instead of automatic selection
results = users.find_with_hint(
    {"age": {"$gte": 25}, "city": "NYC"},
    "users_city"
)
```

**Use hints when:**
- You know the data distribution better than the planner
- Testing index performance
- Working around planner limitations

## Performance

### Index Lookup Complexity

| Operation | Complexity |
|-----------|------------|
| B+ tree lookup | O(log n) |
| Range scan | O(log n + k) |
| Insert with index | O(log n) |
| Delete with index | O(log n) |

Where:
- n = total documents
- k = matching documents

### Memory Usage

- Each index adds memory overhead
- B+ tree nodes cached in memory
- Index size ≈ (key size + 8 bytes) × document count

### Benchmark Results

```
Collection: 100,000 documents

Without index:
  find({age: 25})           32.5ms  (full scan)
  find({age: {$gt: 50}})    45.2ms  (full scan)

With index on age:
  find({age: 25})            0.8ms  (98% faster)
  find({age: {$gt: 50}})     5.2ms  (88% faster)
```

## Best Practices

### 1. Index Fields Used in Queries

```python
# If you frequently query by status:
orders.create_index("status")

# Now this is fast:
orders.find({"status": "pending"})
```

### 2. Index Fields Used in Sort

```python
# If you sort by date:
events.create_index("timestamp")

# Now sorting is efficient:
events.find({}, sort=[("timestamp", -1)])
```

### 3. Use Compound Indexes for Multi-Field Queries

```python
# Instead of two single indexes:
# users.create_index("country")
# users.create_index("city")

# Use one compound index:
users.create_compound_index(["country", "city"])
```

### 4. Consider Field Order in Compound Indexes

Put the most selective field first:

```python
# Good: status has few values, date has many
# Date first allows range queries
logs.create_compound_index(["status", "date"])

# Query: status=error, last 24 hours
logs.find({
    "status": "error",
    "date": {"$gte": yesterday}
})
```

### 5. Don't Over-Index

Each index:
- Increases insert/update/delete time
- Uses memory
- Needs maintenance

**Rule of thumb:** Index fields you query, not fields you store.

### 6. Use Unique Indexes for Constraints

```python
# Enforce unique emails
users.create_index("email", unique=True)

# This will fail if email exists:
users.insert_one({"email": "existing@example.com"})
# Raises: MongoLiteError - Duplicate key
```

### 7. Monitor with explain()

```python
# Check if your query uses indexes
plan = collection.explain({"your": "query"})

if plan["queryPlan"] == "CollectionScan":
    print("Consider adding an index!")
```

## Index Persistence

Indexes are persisted automatically:

- Saved to `.mlite` file with collection metadata
- Rebuilt on database open if corrupted
- Atomic updates with document operations

## Limitations

| Feature | Status |
|---------|--------|
| Text indexes | Not supported |
| Geospatial indexes | Not supported |
| TTL indexes | Not supported |
| Partial indexes | Not supported |
| Index intersection | Not supported |

Supported:
- Single field indexes
- Compound indexes (multi-field)
- Unique constraints
- Ascending order (always)
- Nested field indexes (dot notation)

## API Reference

### Collection Methods

```python
# Create single field index
create_index(field: str, unique: bool = False) -> str

# Create compound index
create_compound_index(fields: List[str], unique: bool = False) -> str

# List all indexes
list_indexes() -> List[str]

# Drop an index
drop_index(index_name: str) -> None

# Explain query plan
explain(query: dict) -> dict

# Query with forced index
find_with_hint(query: dict, index_name: str) -> List[dict]
```

### Index Naming Convention

- Single field: `{collection}_{field}`
- Compound: `{collection}_{field1}_{field2}_...`
- Example: `users_email`, `users_country_city`
