//! Guide prompts - general reference documentation
//!
//! Contains: best-practices, query-operators, aggregation-guide, aggregation-limits, transaction-guide

use serde_json::{json, Value};

pub fn best_practices() -> Value {
    json!({
        "messages": [
            {
                "role": "user",
                "content": {
                    "type": "text",
                    "text": r#"# ⚠️ IronBase Best Practices - READ FIRST

## Critical: Prevent Context Overflow

**NEVER fetch all documents without limits!** Large collections can have thousands of documents.

### ❌ WRONG - Will overflow context:
```json
{"collection": "users", "query": {}}
```

### ✅ CORRECT - Always follow this workflow:

## Step 1: Check Collection Size FIRST
```json
// count_documents tool
{"collection": "users", "query": {}}
```

## Step 2: Use Appropriate Limits
| Collection Size | Recommended Limit |
|-----------------|-------------------|
| < 100 docs | 20-50 |
| 100-1000 docs | 10-20 |
| 1000+ docs | 5-10 |

## Step 3: Use Projection to Reduce Data
Only fetch fields you need:
```json
{
  "collection": "users",
  "query": {},
  "projection": {"name": 1, "email": 1, "_id": 0},
  "limit": 10
}
```

## Step 4: For Large Text Fields
If documents have large text content (articles, descriptions, full_text):
- Use projection to EXCLUDE large fields: `{"full_text": 0, "content": 0}`
- Or include only specific small fields: `{"title": 1, "date": 1}`

## Pagination Pattern
```json
// Page 1
{"collection": "users", "query": {}, "limit": 10, "skip": 0}
// Page 2
{"collection": "users", "query": {}, "limit": 10, "skip": 10}
```

## Safe Aggregation
Always use $limit in pipelines:
```json
[
  {"$match": {"status": "active"}},
  {"$limit": 20},
  {"$project": {"name": 1, "score": 1}}
]
```

### ⚡ Index-Based Count (When Eligible)
For counting by field, index-based optimization is only available when:
- `$group` uses a simple field `_id` (e.g. `"$category"`)
- Accumulators are only `$sum: 1`
- A B+ tree index exists on the group field

When eligible, skipping unnecessary `$match` stages (like `$exists` filters) can help the optimizer use the index path:
```json
// FAST (index path when eligible)
[{"$group": {"_id": "$category", "count": {"$sum": 1}}}, {"$sort": {"count": -1}}, {"$limit": 10}]

// SLOW (full scan)
[{"$match": {"field": {"$exists": true}}}, {"$group": {"_id": "$field", "count": {"$sum": 1}}}]
```

### Memory Limits
Aggregation has built-in OOM protection with limits that can scale by memory tier:
- Default max docs without $match: ~10,000
- Default max docs with $match: ~1,000,000
- Default max unique groups: ~50,000

Exact limits can vary by memory profile. If limit errors occur: add $match filters, use lower-cardinality group keys, or reduce scope.

## Quick Reference

| Operation | Safe Practice |
|-----------|---------------|
| Explore collection | `count_documents` first, then `find` with limit 5-10 |
| Get sample data | `find` with limit 3-5 and projection |
| Search | Use specific filters, limit 10-20 |
| Full-text search | Use `fulltext_search` with limit, projection |
| Aggregation | Always include `$limit` stage |

## Memory Guideline
- Aim for responses under 10KB per query
- Large documents (>1KB each) need smaller limits
- Text fields can be very large - always use projection

**Remember: It's better to make multiple small queries than one large one!**"#
                }
            }
        ]
    })
}
pub fn query_operators() -> Value {
    json!({
        "messages": [
            {
                "role": "user",
                "content": {
                    "type": "text",
                    "text": r#"# IronBase Query Operators Reference

## Comparison Operators
| Operator | Description | Example |
|----------|-------------|---------|
| `$eq` | Equal | `{"age": {"$eq": 25}}` or `{"age": 25}` |
| `$ne` | Not equal | `{"status": {"$ne": "inactive"}}` |
| `$gt` | Greater than | `{"age": {"$gt": 18}}` |
| `$gte` | Greater or equal | `{"score": {"$gte": 90}}` |
| `$lt` | Less than | `{"price": {"$lt": 100}}` |
| `$lte` | Less or equal | `{"count": {"$lte": 10}}` |
| `$in` | In array | `{"city": {"$in": ["NYC", "LA"]}}` |
| `$nin` | Not in array | `{"status": {"$nin": ["deleted", "banned"]}}` |

## Logical Operators
| Operator | Description | Example |
|----------|-------------|---------|
| `$and` | All conditions match | `{"$and": [{"age": {"$gte": 18}}, {"city": "NYC"}]}` |
| `$or` | Any condition matches | `{"$or": [{"city": "NYC"}, {"city": "LA"}]}` |
| `$not` | Negate condition | `{"age": {"$not": {"$gt": 30}}}` |
| `$nor` | None match | `{"$nor": [{"deleted": true}, {"banned": true}]}` |

## Element Operators
| Operator | Description | Example |
|----------|-------------|---------|
| `$exists` | Field exists | `{"email": {"$exists": true}}` |
| `$type` | Type check | `{"age": {"$type": "number"}}` |

## Array Operators
| Operator | Description | Example |
|----------|-------------|---------|
| `$all` | Contains all | `{"tags": {"$all": ["a", "b"]}}` |
| `$elemMatch` | Element matches | `{"scores": {"$elemMatch": {"$gt": 80}}}` |
| `$size` | Array length | `{"tags": {"$size": 3}}` |

## String Operators
| Operator | Description | Example |
|----------|-------------|---------|
| `$regex` | Regex match | `{"name": {"$regex": "^John"}}` |

## Update Operators
| Operator | Description | Example |
|----------|-------------|---------|
| `$set` | Set field | `{"$set": {"name": "Bob"}}` |
| `$inc` | Increment | `{"$inc": {"count": 1}}` |
| `$unset` | Remove field | `{"$unset": {"temp": ""}}` |
| `$push` | Add to array | `{"$push": {"tags": "new"}}` |
| `$pull` | Remove from array | `{"$pull": {"tags": "old"}}` |
| `$addToSet` | Add unique | `{"$addToSet": {"tags": "unique"}}` |
| `$pop` | Remove first/last | `{"$pop": {"items": 1}}` (last) or `-1` (first) |

## Dot Notation (Nested Fields)
Access nested fields using dot notation:
- Query: `{"address.city": "NYC"}`
- Update: `{"$set": {"profile.score": 100}}`
- Sort: `[["stats.rating", -1]]`"#
                }
            }
        ]
    })
}
pub fn aggregation_guide() -> Value {
    json!({
        "messages": [
            {
                "role": "user",
                "content": {
                    "type": "text",
                    "text": r#"# IronBase Aggregation Pipeline Guide

## Pipeline Stages

### $match - Filter documents
```json
{"$match": {"status": "active", "age": {"$gte": 18}}}
```

### $group - Group and aggregate
```json
{
  "$group": {
    "_id": "$city",           // Group by field (use "$fieldname")
    "count": {"$sum": 1},     // Count documents
    "totalSales": {"$sum": "$amount"},
    "avgAge": {"$avg": "$age"},
    "minPrice": {"$min": "$price"},
    "maxPrice": {"$max": "$price"},
    "firstDoc": {"$first": "$name"},
    "lastDoc": {"$last": "$name"},
    "allNames": {"$push": "$name"},      // Collect all values into array
    "uniqueTags": {"$addToSet": "$tag"}  // Collect unique values only
  }
}
```

### $project - Reshape documents
```json
{
  "$project": {
    "_id": 0,                 // Exclude _id
    "fullName": "$name",      // Rename field
    "years": "$age",          // Rename field
    "city": 1,                // Include field
    "tagCount": {"$size": "$tags"},  // Array size
    "total": {"$reduce": {           // Reduce array
      "input": "$items",
      "initialValue": 0,
      "in": {"$add": ["$$value", "$$this.price"]}
    }}
  }
}
```

### $unwind - Deconstruct array field
```json
// Simple form - creates one document per array element
{"$unwind": "$tags"}

// Extended form with options
{"$unwind": {
  "path": "$items",
  "includeArrayIndex": "idx",        // Add index field
  "preserveNullAndEmptyArrays": true // Keep docs with empty/null arrays
}}
```

### $sort - Sort documents
```json
{"$sort": {"count": -1, "name": 1}}
```

### $limit - Limit results
```json
{"$limit": 10}
```

### $skip - Skip documents
```json
{"$skip": 20}
```

## Accumulators (in $group)
| Accumulator | Description | Example |
|-------------|-------------|---------|
| `$sum` | Sum values or count | `{"$sum": 1}` or `{"$sum": "$amount"}` |
| `$avg` | Average | `{"$avg": "$price"}` |
| `$min` | Minimum | `{"$min": "$score"}` |
| `$max` | Maximum | `{"$max": "$score"}` |
| `$first` | First value in group | `{"$first": "$name"}` |
| `$last` | Last value in group | `{"$last": "$name"}` |
| `$push` | Collect ALL values into array | `{"$push": "$item"}` |
| `$addToSet` | Collect UNIQUE values only | `{"$addToSet": "$tag"}` |

## $reduce Expression (in $project)
Reduce an array to a single value with custom logic.

### Sum prices from object array
```json
{"$reduce": {
  "input": "$items",           // Array of objects
  "initialValue": 0,
  "in": {"$add": ["$$value", "$$this.price"]}  // Access object field
}}
```

### Concatenate names with separator
```json
{"$reduce": {
  "input": "$people",
  "initialValue": "",
  "in": {"$concat": ["$$value", ", ", "$$this.name"]}
}}
```

### Multiply factors
```json
{"$reduce": {
  "input": "$factors",
  "initialValue": 1,
  "in": {"$multiply": ["$$value", "$$this"]}
}}
```

**Supported operators in $reduce:**
- `$add` - Sum numbers
- `$multiply` - Multiply numbers
- `$concat` - Concatenate strings (with optional separator)

## Example Pipelines

### Basic grouping with count
```json
[
  {"$match": {"status": "completed"}},
  {"$group": {
    "_id": "$category",
    "totalRevenue": {"$sum": "$amount"},
    "orderCount": {"$sum": 1},
    "avgOrder": {"$avg": "$amount"}
  }},
  {"$sort": {"totalRevenue": -1}},
  {"$limit": 5}
]
```

### Collect items per category
```json
[
  {"$group": {
    "_id": "$category",
    "allItems": {"$push": "$name"},
    "uniqueBrands": {"$addToSet": "$brand"}
  }}
]
```

### Unwind and re-aggregate
```json
[
  {"$unwind": "$items"},
  {"$group": {
    "_id": "$items.category",
    "totalValue": {"$sum": "$items.price"},
    "count": {"$sum": 1}
  }},
  {"$sort": {"totalValue": -1}}
]
```

### Calculate order totals with $reduce
```json
[
  {"$project": {
    "orderId": 1,
    "orderTotal": {"$reduce": {
      "input": "$items",
      "initialValue": 0,
      "in": {"$add": ["$$value", "$$this.price"]}
    }}
  }},
  {"$sort": {"orderTotal": -1}}
]
```

## ⚡ Performance Optimizations

### Index-Based $group (2300x faster!)

For counting by a field that has an index, **skip the $match stage**:

#### ❌ SLOW (284 seconds on 78K docs):
```json
[
  {"$match": {"email": {"$exists": true}}},
  {"$group": {"_id": "$email", "count": {"$sum": 1}}},
  {"$sort": {"count": -1}},
  {"$limit": 5}
]
```

#### ✅ FAST (47 milliseconds on 78K docs):
```json
[
  {"$group": {"_id": "$email", "count": {"$sum": 1}}},
  {"$sort": {"count": -1}},
  {"$limit": 5}
]
```

**Why?** Without `$match`, IronBase uses index-based `$group` optimization - reads only index entries, NOT full documents.

**Requirements for Index-Based $group:**
1. NO leading `$match` stage
2. Single field group key: `{"_id": "$field"}`
3. All accumulators are `$sum: 1` (counting)
4. Single-field index exists on the group field

**When to use $match:** Only when you need to FILTER documents (e.g., `{"status": "active"}`). Skip it for full collection counts.

### Top-K Optimization ($sort + $limit)

When `$sort` is followed by `$limit`, IronBase automatically uses a heap-based algorithm:
- Memory: O(k) instead of O(n)
- Time: O(n log k) instead of O(n log n)

Example (50K groups → only 5 kept in memory):
```json
[
  {"$group": {"_id": "$category", "count": {"$sum": 1}}},
  {"$sort": {"count": -1}},
  {"$limit": 5}
]
```

## Memory Limits (OOM Protection)

IronBase has built-in limits to prevent out-of-memory errors:

| Limit | Default | Purpose |
|-------|---------|---------|
| max_docs_without_match | 100,000 | Max docs to scan without $match |
| max_docs_with_match | 1,000,000 | Max docs even with $match |
| max_group_count | 50,000 | Max unique groups in $group |
| max_push_elements | 100,000 | Max elements per $push accumulator |
| max_memory_mb | 512 | Max estimated memory usage |

**Triggered errors:**
- `"Aggregation exceeded document limit: X documents processed"`
- `"Aggregation exceeded group limit: X unique groups"`

**Solutions:** Add `$match` to filter, use lower-cardinality group key, or use index-based optimization.

### Date Statistics: Aggregation vs Range Queries (90x faster!)

When counting documents by date ranges (e.g., yearly statistics), **range queries are 90x faster** than aggregation with `$substr`.

#### ❌ SLOW - Aggregation with $substr (31 seconds on 78K docs):
```json
[
  {"$project": {"year": {"$substr": ["$date", 0, 4]}}},
  {"$group": {"_id": "$year", "count": {"$sum": 1}}},
  {"$sort": {"_id": 1}}
]
```

**Why slow?** Every document must be loaded and string-processed.

#### ✅ FAST - Range queries with indexed field (346ms on 78K docs):
```json
// Count for 2024 (uses date index)
{"collection": "emails", "query": {"date": {"$gte": "2024", "$lt": "2025"}}}

// Count for 2023
{"collection": "emails", "query": {"date": {"$gte": "2023", "$lt": "2024"}}}
```

**Why fast?** B-tree index range scan - no document loading needed!

#### Performance Comparison (78,295 documents)
| Method | Time | Speedup |
|--------|------|---------|
| Aggregation (`$project` + `$group`) | 31.2s | 1x |
| Range queries (7× `count_documents`) | 346ms | **90x** |

**When to use which:**
- **Range queries**: Known date ranges, indexed date field, need counts only
- **Aggregation**: Dynamic grouping, unknown date values, need other calculations"#
                }
            }
        ]
    })
}

pub fn aggregation_limits() -> Value {
    json!({
        "messages": [
            {
                "role": "user",
                "content": {
                    "type": "text",
                    "text": r#"# Aggregation Memory Limits Guide

## Why Limits Exist

IronBase has built-in memory limits to prevent out-of-memory (OOM) errors on large collections. These limits protect your server from crashing during heavy aggregation operations.

## Dynamic Limits (Recommended)

IronBase automatically calculates limits based on your system's available RAM:

| Available RAM | max_memory_mb | max_docs | max_groups |
|---------------|---------------|----------|------------|
| < 512 MB | 64 | 10,000 | 5,000 |
| 512 MB - 2 GB | 128 | 50,000 | 25,000 |
| 2 GB - 8 GB | 256 | 100,000 | 50,000 |
| 8 GB - 32 GB | 512 | 250,000 | 100,000 |
| > 32 GB | 1,024 | 500,000 | 250,000 |

## Default Static Limits

| Limit | Default Value | Purpose |
|-------|---------------|---------|
| max_docs_without_match | 100,000 | Max documents to scan without $match |
| max_docs_with_match | 1,000,000 | Max documents even WITH $match |
| max_group_count | 50,000 | Max unique groups in $group stage |
| max_push_elements | 100,000 | Max elements per $push accumulator |
| max_addtoset_elements | 100,000 | Max elements per $addToSet accumulator |
| max_unwind_output | 1,000,000 | Max documents after $unwind |
| max_memory_mb | 512 | Max estimated memory usage |

## Common Error Messages

### "Aggregation exceeded document limit: X documents processed"
**Cause:** Too many documents being scanned.
**Solutions:**
1. Add `$match` stage to filter documents first
2. Use index-based aggregation (skip $match for counts)
3. Process in smaller batches

### "Aggregation exceeded group limit: X unique groups"
**Cause:** Group key has too many unique values (high cardinality).
**Solutions:**
1. Group by a lower-cardinality field
2. Add `$match` to reduce input documents
3. Use `$limit` after `$group`

## Best Practices

### 1. Always start with $match (when filtering)
```json
[
  {"$match": {"status": "active", "date": {"$gte": "2024-01-01"}}},
  {"$group": {"_id": "$category", "count": {"$sum": 1}}}
]
```

### 2. Skip $match for full collection counts (2300x faster!)
```json
// Uses index - 47ms on 78K docs
[{"$group": {"_id": "$email", "count": {"$sum": 1}}}, {"$limit": 10}]
```

### 3. Use low-cardinality group keys
- Good: `status`, `category`, `country` (few unique values)
- Bad: `email`, `user_id`, `timestamp` (many unique values)

### 4. Add indexes on $match fields
Create indexes on fields used in `$match` for faster filtering.

### 5. Use $limit after $group
```json
[
  {"$group": {"_id": "$tag", "count": {"$sum": 1}}},
  {"$sort": {"count": -1}},
  {"$limit": 100}  // Only keep top 100
]
```

## Top-K Optimization

When `$sort` is followed by `$limit`, IronBase uses a heap-based algorithm:
- Memory: O(k) instead of O(n)
- Time: O(n log k) instead of O(n log n)

This means sorting 50,000 groups with `$limit: 5` only keeps 5 items in memory!"#
                }
            }
        ]
    })
}

pub fn transaction_guide() -> Value {
    json!({
        "messages": [
            {
                "role": "user",
                "content": {
                    "type": "text",
                    "text": r#"# IronBase Transaction Guide (ACID with Read Committed Isolation)

IronBase supports ACID transactions with Read Committed isolation level and Write-Ahead Logging.

## Transaction Properties (ACID)

| Property | Description |
|----------|-------------|
| **Atomicity** | All operations succeed or all fail |
| **Consistency** | Database remains valid after transaction |
| **Isolation** | Read Committed - no dirty reads, single writer |
| **Durability** | Committed changes survive crashes (WAL) |

## Isolation Behavior (SQLite-style)

- **Only ONE write transaction can be active at a time**
- Second write transaction will **wait** (block) up to 5 seconds for the first to complete
- If timeout expires: "Timeout waiting for write lock after 5s"
- Read operations always see committed data only (no dirty reads)
- Auto-commit operations (insert_one, update_one, etc.) are blocked while a write transaction is active

## Transaction Lifecycle

```
begin_transaction
    ↓
[_tx operations: insert_one_tx, update_one_tx, delete_one_tx]
    ↓
commit_transaction  OR  rollback_transaction
```

## Basic Usage

### 1. Begin Transaction
```json
// begin_transaction tool (no parameters)
{}
```
Returns: `{"transaction_id": "123"}`

### 2. Perform Operations (use _tx variants!)
All _tx operations are buffered until commit:
```json
// insert_one_tx
{"transaction_id": "123", "collection": "accounts", "document": {"id": 1, "balance": 1000}}

// update_one_tx
{"transaction_id": "123", "collection": "accounts", "filter": {"id": 1}, "update": {"$inc": {"balance": -100}}}

// delete_one_tx
{"transaction_id": "123", "collection": "accounts", "filter": {"id": 3}}
```

### 3. Commit or Rollback
```json
// commit_transaction - make changes permanent
{"transaction_id": "123"}

// rollback_transaction - discard all changes
{"transaction_id": "123"}
```

## Available Transaction Tools

| Tool | Description |
|------|-------------|
| `begin_transaction` | Start new transaction, get transaction_id |
| `commit_transaction` | Commit and make changes permanent |
| `rollback_transaction` | Discard all buffered changes |
| `insert_one_tx` | Insert document within transaction |
| `update_one_tx` | Update document within transaction |
| `delete_one_tx` | Delete document within transaction |
| `transaction_status` | Check if write transaction is active |

## Example: Money Transfer

```
1. begin_transaction → {"transaction_id": "123"}
2. Check source balance (use find - reads see committed data)
3. update_one_tx({id: 1}, {$inc: {balance: -100}}, tx_id: "123")
4. update_one_tx({id: 2}, {$inc: {balance: 100}}, tx_id: "123")
5. If all OK: commit_transaction(tx_id: "123")
   If error: rollback_transaction(tx_id: "123")
```

## Read Committed Isolation Example

```
Time  | Transaction 1          | Transaction 2
------|------------------------|------------------
T1    | begin_transaction      |
T2    | insert_one_tx(Alice)   |
T3    |                        | find("users") → [] (Alice not visible!)
T4    | commit_transaction     |
T5    |                        | find("users") → [Alice] (now visible)
```

## Write-Ahead Log (WAL)

- All changes written to WAL before applying
- CRC32 checksums for integrity
- Automatic crash recovery on restart

## Best Practices

1. **Keep transactions short**: Long transactions block ALL other writes
2. **Handle errors**: Always rollback on failure
3. **Use _tx methods**: insert_one_tx, update_one_tx, delete_one_tx within transactions
4. **Check status**: Use transaction_status to see if writes are blocked
5. **Don't mix**: Don't use regular insert_one/update_one during a transaction

## Error Handling

```
tx = begin_transaction
try:
    insert_one_tx(...)
    update_one_tx(...)
    commit_transaction(tx)
except:
    rollback_transaction(tx)
```

## Limitations

- Only ONE write transaction at a time (SQLite-style)
- Second write transaction waits up to 5 seconds (blocking)
- Regular CRUD operations blocked during active write transaction
- No nested transactions
- No distributed transactions"#
                }
            }
        ]
    })
}
