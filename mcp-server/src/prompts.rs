//! MCP Prompt definitions for IronBase

use serde_json::{json, Value};

/// Get the list of all available prompts for MCP prompts/list
pub fn get_prompts_list() -> Value {
    json!({
        "prompts": [
            {
                "name": "discover-schema",
                "description": "Analyze a collection's structure by examining sample documents to understand field types, nesting, and common patterns",
                "arguments": [
                    {
                        "name": "collection",
                        "description": "The collection name to analyze",
                        "required": true
                    },
                    {
                        "name": "sample_size",
                        "description": "Number of documents to sample (default: 10)",
                        "required": false
                    }
                ]
            },
            {
                "name": "query-operators",
                "description": "Reference guide for all IronBase query operators with examples",
                "arguments": []
            },
            {
                "name": "aggregation-guide",
                "description": "Guide for building aggregation pipelines with examples of $match, $group, $project, $sort, and accumulators",
                "arguments": []
            },
            {
                "name": "query-examples",
                "description": "Common query patterns and examples for typical use cases",
                "arguments": [
                    {
                        "name": "category",
                        "description": "Query category: 'crud', 'aggregation', 'indexes', or 'all'",
                        "required": false
                    }
                ]
            },
            {
                "name": "date-query",
                "description": "Help building date range queries from natural language expressions like 'yesterday', 'last week', 'this month'",
                "arguments": [
                    {
                        "name": "date_expression",
                        "description": "Natural language date expression (e.g., 'yesterday', 'last 7 days', 'Q1 2024')",
                        "required": true
                    },
                    {
                        "name": "date_field",
                        "description": "Name of the date field in documents",
                        "required": true
                    }
                ]
            },
            {
                "name": "wildcard-operator",
                "description": "Guide for using the $** recursive descent wildcard operator to find fields at any nesting depth",
                "arguments": []
            },
            {
                "name": "schema-validation",
                "description": "Guide for setting up JSON schema validation on collections to enforce document structure",
                "arguments": [
                    {
                        "name": "collection",
                        "description": "The collection name to set schema on",
                        "required": true
                    }
                ]
            },
            {
                "name": "index-optimization",
                "description": "Guide for optimizing queries with indexes, including when to use single-field vs compound indexes",
                "arguments": [
                    {
                        "name": "collection",
                        "description": "The collection to analyze and optimize",
                        "required": true
                    }
                ]
            },
            {
                "name": "transaction-guide",
                "description": "Guide for using ACD transactions with begin, commit, and rollback operations",
                "arguments": []
            }
        ]
    })
}

/// Get prompt content by name
pub fn get_prompt_content(name: &str, arguments: &Value) -> Option<Value> {
    match name {
        "discover-schema" => Some(get_discover_schema_prompt(arguments)),
        "query-operators" => Some(get_query_operators_prompt()),
        "aggregation-guide" => Some(get_aggregation_guide_prompt()),
        "query-examples" => Some(get_query_examples_prompt(arguments)),
        "date-query" => Some(get_date_query_prompt(arguments)),
        "wildcard-operator" => Some(get_wildcard_operator_prompt()),
        "schema-validation" => Some(get_schema_validation_prompt(arguments)),
        "index-optimization" => Some(get_index_optimization_prompt(arguments)),
        "transaction-guide" => Some(get_transaction_guide_prompt()),
        _ => None,
    }
}

fn get_discover_schema_prompt(arguments: &Value) -> Value {
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

fn get_query_operators_prompt() -> Value {
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

fn get_aggregation_guide_prompt() -> Value {
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
    "lastDoc": {"$last": "$name"}
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
    "city": 1                 // Include field
  }
}
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
| Accumulator | Description |
|-------------|-------------|
| `$sum` | Sum values or count (`{"$sum": 1}`) |
| `$avg` | Average |
| `$min` | Minimum |
| `$max` | Maximum |
| `$first` | First value in group |
| `$last` | Last value in group |

## Example Pipeline
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
```"#
                }
            }
        ]
    })
}

fn get_query_examples_prompt(arguments: &Value) -> Value {
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

## List indexes
```json
{"collection": "users"}
```

## Use explain to check index usage
Use the find tool with explain to see if indexes are being used."#
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

fn get_date_query_prompt(arguments: &Value) -> Value {
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

fn get_wildcard_operator_prompt() -> Value {
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

fn get_schema_validation_prompt(arguments: &Value) -> Value {
    let collection = arguments
        .get("collection")
        .and_then(|v| v.as_str())
        .unwrap_or("your_collection");

    json!({
        "messages": [
            {
                "role": "user",
                "content": {
                    "type": "text",
                    "text": format!(r#"# JSON Schema Validation Guide

## Setting Schema on Collection: "{}"

Use the `schema_set` tool to enforce document structure.

## Basic Schema Example
```json
{{
  "collection": "{}",
  "schema": {{
    "type": "object",
    "required": ["name", "email"],
    "properties": {{
      "name": {{"type": "string", "minLength": 1}},
      "email": {{"type": "string", "format": "email"}},
      "age": {{"type": "integer", "minimum": 0}},
      "active": {{"type": "boolean"}}
    }}
  }}
}}
```

## Schema with Nested Objects
```json
{{
  "type": "object",
  "properties": {{
    "user": {{
      "type": "object",
      "required": ["id"],
      "properties": {{
        "id": {{"type": "string"}},
        "profile": {{
          "type": "object",
          "properties": {{
            "name": {{"type": "string"}},
            "bio": {{"type": "string", "maxLength": 500}}
          }}
        }}
      }}
    }}
  }}
}}
```

## Schema with Arrays
```json
{{
  "type": "object",
  "properties": {{
    "tags": {{
      "type": "array",
      "items": {{"type": "string"}},
      "minItems": 1,
      "uniqueItems": true
    }},
    "scores": {{
      "type": "array",
      "items": {{"type": "number", "minimum": 0, "maximum": 100}}
    }}
  }}
}}
```

## Validation Types
| Type | Description |
|------|-------------|
| `string` | Text values |
| `number` | Any numeric value |
| `integer` | Whole numbers only |
| `boolean` | true/false |
| `array` | List of items |
| `object` | Nested document |
| `null` | Null value |

## String Constraints
- `minLength`, `maxLength`: Length limits
- `pattern`: Regex pattern
- `format`: email, uri, date-time, etc.

## Number Constraints
- `minimum`, `maximum`: Value range
- `exclusiveMinimum`, `exclusiveMaximum`: Exclusive range

## Array Constraints
- `minItems`, `maxItems`: Array length
- `uniqueItems`: No duplicates

## Tools
- `schema_set`: Set/update schema
- `schema_get`: View current schema
- `schema_delete`: Remove validation"#, collection, collection)
                }
            }
        ]
    })
}

fn get_index_optimization_prompt(arguments: &Value) -> Value {
    let collection = arguments
        .get("collection")
        .and_then(|v| v.as_str())
        .unwrap_or("your_collection");

    json!({
        "messages": [
            {
                "role": "user",
                "content": {
                    "type": "text",
                    "text": format!(r#"# Index Optimization Guide

## Collection: "{}"

## Creating Indexes

### Single-Field Index
```json
// create_index tool
{{"collection": "{}", "field": "email", "unique": true}}
```

### Compound Index (multiple fields)
```json
// create_compound_index tool
{{"collection": "{}", "fields": ["country", "city", "created_at"]}}
```

## When to Create Indexes

| Query Pattern | Recommended Index |
|---------------|-------------------|
| `{{"email": "x"}}` | Single on `email` |
| `{{"country": "x", "city": "y"}}` | Compound `[country, city]` |
| `{{"status": "x"}}` + sort by `date` | Compound `[status, date]` |
| `{{"age": {{"$gte": 18}}}}` | Single on `age` |

## Index Selection Rules

1. **Equality first**: Put exact match fields before range fields
   - Good: `[status, created_at]` for `{{status: "active", created_at: {{$gte: ...}}}}`
   - Bad: `[created_at, status]`

2. **Sort field last**: If sorting, include sort field at end
   - Query: `{{status: "active"}}` + sort by `name`
   - Index: `[status, name]`

3. **Selectivity matters**: More selective fields first
   - `user_id` (unique) before `status` (few values)

## Checking Index Usage

Use the `explain` parameter to see query plan:
```json
{{"collection": "{}", "query": {{"status": "active"}}, "explain": true}}
```

## Index Hints

Force specific index usage:
```json
{{"collection": "{}", "query": {{}}, "hint": "status_1"}}
```

## Listing Indexes
```json
// list_indexes tool
{{"collection": "{}"}}
```

## Dropping Indexes
```json
// drop_index tool
{{"collection": "{}", "index_name": "email_1"}}
```

## Performance Tips

| Scenario | Recommendation |
|----------|----------------|
| High write volume | Fewer indexes |
| Read-heavy | More indexes OK |
| Large collections | Essential for performance |
| Small collections (<1000) | May not need indexes |

## Limitations
- `$**` wildcard queries cannot use indexes
- `$or` queries may not use compound indexes efficiently
- `$regex` without anchor (^) cannot use index"#,
                        collection, collection, collection, collection, collection, collection, collection)
                }
            }
        ]
    })
}

fn get_transaction_guide_prompt() -> Value {
    json!({
        "messages": [
            {
                "role": "user",
                "content": {
                    "type": "text",
                    "text": r#"# IronBase Transaction Guide (ACD)

IronBase supports ACD (Atomicity, Consistency, Durability) transactions with Write-Ahead Logging.

## Transaction Lifecycle

```
begin_transaction
    ↓
[operations: insert, update, delete]
    ↓
commit_transaction  OR  rollback_transaction
```

## Basic Usage

### 1. Begin Transaction
```json
// begin_transaction tool (no parameters)
{}
```
Returns: `{"transaction_id": "uuid-here"}`

### 2. Perform Operations
All operations within transaction are isolated:
```json
// insert_one
{"collection": "accounts", "document": {"id": 1, "balance": 1000}}

// update_one
{"collection": "accounts", "filter": {"id": 1}, "update": {"$inc": {"balance": -100}}}

// update_one
{"collection": "accounts", "filter": {"id": 2}, "update": {"$inc": {"balance": 100}}}
```

### 3. Commit or Rollback
```json
// commit_transaction - make changes permanent
{}

// rollback_transaction - discard all changes
{}
```

## Example: Money Transfer

```
1. begin_transaction
2. Check source account balance
3. Deduct from source: update_one({id: 1}, {$inc: {balance: -100}})
4. Add to destination: update_one({id: 2}, {$inc: {balance: 100}})
5. If all OK: commit_transaction
   If error: rollback_transaction
```

## Transaction Properties

| Property | Description |
|----------|-------------|
| **Atomicity** | All operations succeed or all fail |
| **Consistency** | Database remains valid after transaction |
| **Durability** | Committed changes survive crashes (WAL) |

## Write-Ahead Log (WAL)

- All changes written to WAL before applying
- CRC32 checksums for integrity
- Automatic crash recovery on restart

## Durability Modes

| Mode | Description | Use Case |
|------|-------------|----------|
| Safe | Sync after each write | Critical data |
| Batch | Sync periodically | Better performance |
| Unsafe | No sync | Testing only |

## Best Practices

1. **Keep transactions short**: Long transactions block other operations
2. **Handle errors**: Always rollback on failure
3. **Don't nest**: One transaction at a time per connection
4. **Use for related changes**: Group logically related operations

## Error Handling

```
begin_transaction
try:
    // operations
    commit_transaction
except:
    rollback_transaction
```

## Limitations

- Single-database transactions only
- No distributed transactions
- One active transaction per connection
- Long transactions may impact performance"#
                }
            }
        ]
    })
}
