//! Query-related prompts
//!
//! Contains: discover-schema, query-examples, date-query, wildcard-operator

use serde_json::{json, Value};

pub fn discover_schema(arguments: &Value) -> Value {
    let collection = arguments
        .get("collection")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    let sample_size = arguments
        .get("sample_size")
        .and_then(|v| v.as_u64())
        .unwrap_or(10);

    json!({
        "messages": [
            {
                "role": "user",
                "content": {
                    "type": "text",
                    "text": format!(
                        "Analyze the schema of the '{}' collection by sampling {} documents.\n\n\
                        Use the 'find' tool to retrieve sample documents:\n\
                        ```json\n\
                        {{\n\
                          \"collection\": \"{}\",\n\
                          \"query\": {{}},\n\
                          \"limit\": {}\n\
                        }}\n\
                        ```\n\n\
                        Then provide a summary including:\n\
                        1. All field names and their types\n\
                        2. Nested object structure\n\
                        3. Array fields and their element types\n\
                        4. Optional vs required fields (based on presence)\n\
                        5. Suggested indexes based on likely query patterns",
                        collection, sample_size, collection, sample_size
                    )
                }
            }
        ]
    })
}

pub fn query_examples(arguments: &Value) -> Value {
    let category = arguments
        .get("category")
        .and_then(|v| v.as_str())
        .unwrap_or("all");

    let content = match category {
        "crud" => {
            r#"# CRUD Query Examples

## Insert
```json
// insert_one
{"collection": "users", "document": {"name": "Alice", "age": 30, "city": "NYC"}}

// insert_many
{"collection": "users", "documents": [
  {"name": "Bob", "age": 25},
  {"name": "Carol", "age": 35}
]}
```

## Find
```json
// Find all
{"collection": "users", "query": {}}

// Find with filter
{"collection": "users", "query": {"city": "NYC"}}

// Find with options
{"collection": "users", "query": {"age": {"$gte": 18}}, "sort": [["age", -1]], "limit": 10}

// Find with projection
{"collection": "users", "query": {}, "projection": {"name": 1, "age": 1, "_id": 0}}
```

## Update
```json
// Update one
{"collection": "users", "filter": {"name": "Alice"}, "update": {"$set": {"age": 31}}}

// Increment
{"collection": "users", "filter": {"name": "Alice"}, "update": {"$inc": {"score": 10}}}
```

## Delete
```json
// Delete one
{"collection": "users", "filter": {"name": "Bob"}}

// Delete many
{"collection": "users", "filter": {"status": "inactive"}}
```"#
        }
        "aggregation" => {
            r#"# Aggregation Examples

## Count by category
```json
{"collection": "products", "pipeline": [
  {"$group": {"_id": "$category", "count": {"$sum": 1}}},
  {"$sort": {"count": -1}}
]}
```

## Calculate totals
```json
{"collection": "orders", "pipeline": [
  {"$match": {"status": "completed"}},
  {"$group": {
    "_id": "$customer_id",
    "totalSpent": {"$sum": "$amount"},
    "orderCount": {"$sum": 1}
  }},
  {"$sort": {"totalSpent": -1}},
  {"$limit": 10}
]}
```

## Date-based aggregation
```json
{"collection": "sales", "pipeline": [
  {"$match": {"date": {"$gte": "2024-01-01"}}},
  {"$group": {
    "_id": "$product",
    "revenue": {"$sum": "$amount"},
    "avgPrice": {"$avg": "$price"}
  }}
]}
```"#
        }
        "indexes" => {
            r#"# Index Examples

## Create single-field index
```json
{"collection": "users", "field": "email", "unique": true}
```

## Create compound index
```json
{"collection": "users", "fields": ["city", "age"]}
```

## Create sparse index (for optional fields)
```json
// Only indexes documents where 'attachments' exists
{"collection": "emails", "field": "attachments", "sparse": true}

// Query using sparse index - O(k) instead of O(n)!
{"collection": "emails", "query": {"attachments": {"$exists": true}}}
```

Sparse indexes are perfect for:
- Optional fields (attachments, metadata, premium_features)
- Soft-delete patterns (deletedAt field)
- Partial data (verifiedAt, completedAt)

## List indexes
```json
{"collection": "users"}
```

## Use explain to check index usage
```json
// explain tool - shows SparseIndexScan for $exists queries
{"collection": "emails", "query": {"attachments": {"$exists": true}}}
```"#
        }
        _ => {
            r#"# IronBase Query Examples

## CRUD Operations
- insert_one/insert_many: Add documents
- find/find_one: Query documents
- update_one/update_many: Modify documents
- delete_one/delete_many: Remove documents

## Aggregation
- Use aggregate tool with pipeline stages
- Stages: $match, $group, $project, $sort, $limit, $skip
- Accumulators: $sum, $avg, $min, $max, $first, $last

## Indexes
- Create indexes for frequently queried fields
- Use compound indexes for multi-field queries
- Check query plans with explain"#
        }
    };

    json!({
        "messages": [
            {
                "role": "user",
                "content": {
                    "type": "text",
                    "text": content
                }
            }
        ]
    })
}

pub fn date_query(arguments: &Value) -> Value {
    let date_expr = arguments
        .get("date_expression")
        .and_then(|v| v.as_str())
        .unwrap_or("today");
    let date_field = arguments
        .get("date_field")
        .and_then(|v| v.as_str())
        .unwrap_or("date");

    json!({
        "messages": [
            {
                "role": "user",
                "content": {
                    "type": "text",
                    "text": format!(
                        r#"Build a date query for the expression: "{}"
Field name: "{}"

## Date Query Patterns

### ISO 8601 format (recommended)
Store dates as ISO strings: "2024-01-15T10:30:00Z"

### Yesterday
```json
{{
  "{}": {{
    "$gte": "YYYY-MM-DDT00:00:00Z",
    "$lt": "YYYY-MM-DDT00:00:00Z"
  }}
}}
```

### Last N days
```json
{{
  "{}": {{
    "$gte": "START_DATE",
    "$lte": "END_DATE"
  }}
}}
```

### This month
```json
{{
  "{}": {{
    "$gte": "YYYY-MM-01T00:00:00Z",
    "$lt": "YYYY-MM+1-01T00:00:00Z"
  }}
}}
```

Please calculate the actual date values for "{}" and provide the complete query."#,
                        date_expr, date_field, date_field, date_field, date_field, date_expr
                    )
                }
            }
        ]
    })
}

pub fn wildcard_operator() -> Value {
    json!({
        "messages": [
            {
                "role": "user",
                "content": {
                    "type": "text",
                    "text": r#"# $** Wildcard Operator (Recursive Descent)

The `$**` operator finds a field name at ANY depth in the document structure.

## Syntax
```
{"$**.fieldName": value}
{"$**.fieldName": {"$operator": value}}
```

## Examples

### Simple match - find "name" at any depth
```json
// Document: {"user": {"profile": {"name": "Alice"}}}
{"$**.name": "Alice"}  // ✅ Matches
```

### With regex - search content anywhere
```json
// Find documents where ANY "content" field contains "sqrt"
{"$**.content": {"$regex": "sqrt"}}
```

### With comparison operators
```json
// Find where ANY "score" field >= 85
{"$**.score": {"$gte": 85}}

// Find where ANY "status" is in list
{"$**.status": {"$in": ["active", "pending"]}}
```

### Multiple matches
```json
// Document: {"a": {"name": "x"}, "b": {"name": "y"}}
{"$**.name": "x"}  // ✅ Matches (finds first "name")
```

### Arrays - searches inside array elements
```json
// Document: {"items": [{"eid": "123"}, {"eid": "456"}]}
{"$**.eid": "123"}  // ✅ Matches
```

## Limitations

| Feature | Supported |
|---------|-----------|
| Simple field name | ✅ `$**.name` |
| Nested paths | ❌ `$**.a.b` is INVALID |
| Index usage | ❌ Always collection scan |
| Max depth | 100 levels (DoS protection) |

## Performance
- ~5% overhead vs dot notation
- ~50 ns per document (file storage)
- Linear O(n) scaling with collection size

## When to Use
- Unknown document structure
- Searching across varied schemas
- Finding fields in deeply nested data
- When exact path is not known

## When NOT to Use
- Known, fixed schema (use dot notation)
- Performance-critical queries (cannot use indexes)
- Very large collections without filtering"#
                }
            }
        ]
    })
}
