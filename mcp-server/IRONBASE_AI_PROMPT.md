# IronBase MCP Server - AI Usage Guide

Use this prompt to help AI assistants correctly interact with IronBase MCP server.

## Critical Rules

### 1. ALWAYS Check Collection Size First
```json
// FIRST: count_documents
{"collection": "emails", "query": {}}

// THEN: find with limit
{"collection": "emails", "query": {}, "limit": 10}
```

### 2. Use Projection for Large Documents
```json
{
  "collection": "emails",
  "query": {},
  "projection": {"subject": 1, "from": 1, "_id": 1},
  "limit": 10
}
```

### 3. Fulltext Search - Field Name Only (NO suffix!)
```json
// ✅ CORRECT - use field name only
{"collection": "emails", "field": "subject", "query": "meeting", "limit": 10}

// ❌ WRONG - don't add _fts suffix
{"collection": "emails", "field": "subject_fts", "query": "meeting"}
```

### 4. Distinct - Use "query" Not "filter"
```json
// ✅ CORRECT
{"collection": "emails", "field": "from.email", "query": {"status": "sent"}}

// ❌ WRONG
{"collection": "emails", "field": "from.email", "filter": {"status": "sent"}}
```

## Common Errors and Solutions

| Error | Cause | Solution |
|-------|-------|----------|
| `Unknown operator: $concat` | Expression operators not supported | Use Rhai script for string manipulation |
| `Group _id must use supported operator` | Complex _id expression | Use simple field reference: `"_id": "$field"` |
| `For loop expects iterable type` | Rhai script error | `db_find()` returns object with `.documents` array - iterate over that |
| `No fulltext index found for field 'x_fts'` | Wrong field name | Use original field name without `_fts` suffix |
| `Aggregation timed out` | Query too slow (>60s) | Add `$match` stage first, use indexes, add `$limit` |
| `Script exceeded maximum operations limit` | Infinite loop or >1M operations | Reduce iterations, use `max_operations` param |
| `Unknown property 'documents' on array` | Already have array, not object | `db_aggregate()` returns array directly, no `.documents` needed |
| `Data type incorrect: i64 expecting string` | Rhai type mismatch | Use explicit conversion: `value.to_string()` |

## Supported Query Operators

| Category | Operators |
|----------|-----------|
| Comparison | `$eq`, `$ne`, `$gt`, `$gte`, `$lt`, `$lte`, `$in`, `$nin` |
| Logical | `$and`, `$or`, `$not`, `$nor` |
| Element | `$exists`, `$type` |
| Array | `$all`, `$elemMatch`, `$size` |
| String | `$regex` |
| Special | `$fuzzy`, `$**` (wildcard) |

## Supported Aggregation

### Stages
`$match`, `$group`, `$project`, `$sort`, `$limit`, `$skip`, `$unwind`, `$count`

### Accumulators (in $group)
`$sum`, `$avg`, `$min`, `$max`, `$first`, `$last`, `$push`, `$addToSet`

### NOT Supported
- Expression operators: `$concat`, `$substr`, `$toUpper`, `$toLower`
- Date operators: `$year`, `$month`, `$dayOfMonth`
- Computed `_id` in `$group` (use simple field reference)

## Aggregation Examples

### ✅ Working: Top senders
```json
[
  {"$group": {"_id": "$from.email", "count": {"$sum": 1}}},
  {"$sort": {"count": -1}},
  {"$limit": 10}
]
```

### ✅ Working: Filter then group
```json
[
  {"$match": {"status": "sent"}},
  {"$group": {"_id": "$category", "total": {"$sum": "$amount"}}}
]
```

### ❌ NOT Working: Computed _id
```json
[
  {"$group": {"_id": {"$concat": ["$first", "$last"]}, "count": {"$sum": 1}}}
]
```

## Rhai Script Best Practices

### Function Return Types (CRITICAL!)
```rhai
// db_find() returns OBJECT with .documents array
let result = db_find("users", #{});
for doc in result.documents {  // ← .documents required!
    print(doc.name);
}

// db_aggregate() returns ARRAY directly
let results = db_aggregate("users", pipeline);
for doc in results {  // ← NO .documents needed!
    print(doc._id);
}

// db_find_one() returns document or null
let doc = db_find_one("users", #{name: "Alice"});
```

### ❌ Common Mistakes
```rhai
// WRONG - db_find returns object, not array
for doc in db_find("users", #{}) { ... }

// WRONG - db_aggregate returns array, not object
let results = db_aggregate("users", pipeline);
for doc in results.documents { ... }  // Error: 'documents' not on array
```

### Handle null/error
```rhai
let doc = db_find_one("users", #{name: "Alice"});
if is_null(doc) {
    "Not found"
} else if is_error(doc) {
    "Error: " + get_error(doc)
} else {
    doc.name
}
```

### Query operators in Rhai
```rhai
// Use backticks for $ operators
let query = #{age: #{`$gt`: 18, `$lt`: 65}};
let result = db_find("users", query);
```

## Tool Availability

| Tool | Description |
|------|-------------|
| `find` | Query documents with filter, projection, sort, limit, skip |
| `find_one` | Get single document |
| `count_documents` | Count matching documents |
| `distinct` | Get unique field values |
| `aggregate` | Run aggregation pipeline |
| `fulltext_search` | TF-IDF text search (requires fulltext index) |
| `fuzzy_search` | Fuzzy text matching (requires fuzzy index) |
| `explain` | Show query execution plan |
| `script_exec` | Run Rhai script inline |
| `script_run` | Run saved script by name |
| `fulltext_analyze` | Debug tokenization (original → normalized → stemmed) |
| `index_stats_refresh` | Rebuild index stats for query planner (after bulk insert) |

## Fulltext Search with Highlighting

### Basic search
```json
{"collection": "articles", "field": "content", "query": "database", "limit": 10}
```

### Enable highlighting (returns `<mark>matched</mark>` snippets)
```json
{
  "collection": "articles",
  "field": "content",
  "query": "database",
  "limit": 10,
  "highlight": true,
  "projection": {"content": 1, "title": 1}
}
```

### Custom highlight settings
```json
{
  "highlight": true,
  "highlight_context": 150,
  "highlight_max_snippets": 5
}
```

| Parameter | Default | Range | Description |
|-----------|---------|-------|-------------|
| `highlight` | false | - | Enable `<mark>` snippets |
| `highlight_context` | 100 | 20-500 | Characters around match |
| `highlight_max_snippets` | 3 | 1-10 | Max snippets per field |

**Note:** Searched field MUST be in projection for highlights to work.

## Query Planner Optimization

After bulk inserts (100k+ docs), refresh index statistics:
```json
{"name": "index_stats_refresh", "arguments": {"collection": "big_table"}}
```

This rebuilds equi-depth histograms (64 buckets) for better range query selectivity estimation.

## Index Naming Convention

| Index Type | Name Format | Example |
|------------|-------------|---------|
| Single field | `{collection}_{field}` | `emails_subject` |
| Compound | `{collection}_{field1}_{field2}` | `emails_from_date` |
| Fulltext | `{collection}_{field}_fts` | `emails_body_fts` |
| Fuzzy | `{collection}_{field}_fuzzy` | `emails_name_fuzzy` |

## Response Size Limits

- Default max response: ~100 MB (RAM-based scaling)
- Use `limit` and `projection` to stay under limits
- Large responses trigger: `"Response size limit exceeded..."`

## Performance Expectations

| Operation | Typical Time | Warning Threshold |
|-----------|--------------|-------------------|
| `find` (indexed) | 1-10ms | >100ms |
| `find` (scan) | 100-500ms | >1s |
| `aggregate` (simple) | 50-100ms | >500ms |
| `aggregate` (complex) | 100-500ms | >5s |
| `fulltext_search` | 300-2000ms | >4s |
| `fuzzy_search` | 200-500ms | >1s |
| `script_exec` | 100-5000ms | >30s |
| `count_documents` | 0-10ms | >100ms |

**Timeout:** 60 seconds for all operations.

### Performance Tips
- Add `$match` FIRST in aggregation pipelines
- Use `limit` on find queries
- Create indexes for frequently queried fields
- Use projection to reduce response size
- For scripts: keep iterations under 100k

## Quick Debugging

```json
// Check server status
{"name": "db_stats", "arguments": {}}

// List all tools
{"method": "tools/list"}

// Explain slow query
{"name": "explain", "arguments": {"collection": "x", "query": {...}}}

// Debug tokenization (why fulltext search misses)
{"name": "fulltext_analyze", "arguments": {"text": "your query", "language": "hungarian"}}
```
