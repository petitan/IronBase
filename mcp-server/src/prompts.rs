//! MCP Prompt definitions for IronBase

use serde_json::{json, Value};

/// Get the list of all available prompts for MCP prompts/list
pub fn get_prompts_list() -> Value {
    json!({
        "prompts": [
            {
                "name": "best-practices",
                "description": "⚠️ IMPORTANT: Read this FIRST before querying data. Guidelines for safe, efficient database operations to avoid context overflow.",
                "arguments": []
            },
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
            },
            {
                "name": "rhai-scripting",
                "description": "Guide for writing Rhai scripts with all available functions, helpers, and best practices",
                "arguments": []
            },
            {
                "name": "fuzzy-search",
                "description": "Guide for fuzzy text search with Jaro-Winkler, Levenshtein, and Damerau-Levenshtein algorithms",
                "arguments": [
                    {
                        "name": "field",
                        "description": "The field name to search (optional, for examples)",
                        "required": false
                    }
                ]
            },
            {
                "name": "fulltext-search",
                "description": "Guide for full-text search with TF-IDF scoring, stemming, and multi-language support (Hungarian, English, German)",
                "arguments": [
                    {
                        "name": "language",
                        "description": "Language for stemming: 'hungarian', 'english', 'german', or 'none'",
                        "required": false
                    }
                ]
            },
            {
                "name": "acl-guide",
                "description": "Guide for Access Control Lists - collection-level permissions based on client origin (localhost, internal, external)",
                "arguments": []
            },
            {
                "name": "database-admin",
                "description": "Guide for database administration: db_open, db_compact, db_checkpoint, db_stats, and backup operations",
                "arguments": []
            },
            {
                "name": "security-guide",
                "description": "Comprehensive security guide covering API keys, ACL, TLS/HTTPS configuration",
                "arguments": []
            },
            {
                "name": "listener-config",
                "description": "Guide for configuring multiple HTTP/HTTPS listeners with different interfaces and TLS settings",
                "arguments": []
            }
        ]
    })
}

/// Get prompt content by name
pub fn get_prompt_content(name: &str, arguments: &Value) -> Option<Value> {
    match name {
        "best-practices" => Some(get_best_practices_prompt()),
        "discover-schema" => Some(get_discover_schema_prompt(arguments)),
        "query-operators" => Some(get_query_operators_prompt()),
        "aggregation-guide" => Some(get_aggregation_guide_prompt()),
        "query-examples" => Some(get_query_examples_prompt(arguments)),
        "date-query" => Some(get_date_query_prompt(arguments)),
        "wildcard-operator" => Some(get_wildcard_operator_prompt()),
        "schema-validation" => Some(get_schema_validation_prompt(arguments)),
        "index-optimization" => Some(get_index_optimization_prompt(arguments)),
        "transaction-guide" => Some(get_transaction_guide_prompt()),
        "rhai-scripting" => Some(get_rhai_scripting_prompt()),
        "fuzzy-search" => Some(get_fuzzy_search_prompt(arguments)),
        "fulltext-search" => Some(get_fulltext_search_prompt(arguments)),
        "acl-guide" => Some(get_acl_guide_prompt()),
        "database-admin" => Some(get_database_admin_prompt()),
        "security-guide" => Some(get_security_guide_prompt()),
        "listener-config" => Some(get_listener_config_prompt()),
        _ => None,
    }
}

fn get_best_practices_prompt() -> Value {
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
{"collection": "users", "filter": {}}
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

4. **Sparse indexes for optional fields**: Use `sparse: true` for fields that don't exist in all documents
   - Optimizes `$exists: true` queries from O(n) to O(k)
   - Only indexes documents where the field exists

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

fn get_rhai_scripting_prompt() -> Value {
    json!({
        "messages": [
            {
                "role": "user",
                "content": {
                    "type": "text",
                    "text": r#"# IronBase Rhai Scripting Guide

Rhai is a lightweight scripting language for server-side operations. Scripts can be saved and reused, with version history and execution statistics.

## Running Scripts

### Execute Inline Code
```json
// script_exec tool
{"code": "let x = 1 + 2; x", "params": {"name": "test"}}
```

### Run Saved Script
```json
// script_run tool
{"name": "my_script", "params": {"count": 10}}

// With custom operation limit (DoS protection)
{"name": "my_script", "params": {}, "max_operations": 500000}
```

## Database Functions

| Function | Description | Example |
|----------|-------------|---------|
| `db_find(coll, query)` | Find documents | `db_find("users", #{age: #{`$gt`: 18}})` |
| `db_find_one(coll, query)` | Find single document (or null) | `db_find_one("users", #{name: "Alice"})` |
| `db_find_one_result(coll, query)` | Find with explicit result type | Returns `#{found: bool, doc: ..., error: ...}` |
| `db_insert_one(coll, doc)` | Insert document | `db_insert_one("users", #{name: "Bob"})` |
| `db_insert_many(coll, docs)` | Insert multiple | `db_insert_many("users", [#{...}, #{...}])` |
| `db_update_one(coll, filter, update)` | Update one | `db_update_one("users", #{name: "x"}, #{`$set`: #{age: 30}})` |
| `db_update_many(coll, filter, update)` | Update many | Same as update_one |
| `db_delete_one(coll, filter)` | Delete one | `db_delete_one("users", #{name: "x"})` |
| `db_delete_many(coll, filter)` | Delete many | Same as delete_one |
| `db_count(coll, query)` | Count documents | `db_count("users", #{active: true})` |
| `db_aggregate(coll, pipeline)` | Aggregation | `db_aggregate("sales", [#{`$group`: ...}])` |

## Index Management Functions

| Function | Description | Example |
|----------|-------------|---------|
| `db_create_index(coll, field, unique)` | Create single-field index | `db_create_index("users", "email", true)` |
| `db_create_compound_index(coll, fields, unique)` | Create compound index | `db_create_compound_index("orders", ["user_id", "date"], false)` |
| `db_list_indexes(coll)` | List all indexes | `db_list_indexes("users")` |
| `db_drop_index(coll, name)` | Drop index by name | `db_drop_index("users", "email_1")` |
| `db_explain(coll, query)` | Explain query plan | `db_explain("users", #{age: #{`$gt`: 18}})` |
| `db_distinct(coll, field, query)` | Get distinct values | `db_distinct("users", "city", #{active: true})` |

## Search Functions

| Function | Description | Example |
|----------|-------------|---------|
| `db_create_fuzzy_index(coll, field, algo, threshold)` | Create fuzzy index | `db_create_fuzzy_index("users", "name", "jaro_winkler", 0.8)` |
| `db_fuzzy_search(coll, field, query, threshold)` | Fuzzy text search | `db_fuzzy_search("users", "name", "john", 0.7)` |
| `db_create_fulltext_index(coll, field, lang)` | Create fulltext index | `db_create_fulltext_index("articles", "content", "english")` |
| `db_fulltext_search(coll, field, query, limit)` | Fulltext search with TF-IDF | `db_fulltext_search("articles", "content", "database", 10)` |

## Helper Functions

| Function | Description | Example |
|----------|-------------|---------|
| `is_error(v)` | Check if value is error string | `if is_error(result) { ... }` |
| `is_null(v)` | Check if value is null/unit | `if is_null(doc) { ... }` |
| `get_error(v)` | Extract error message | `let msg = get_error(result);` |
| `unwrap_or(v, default)` | Return value or default if error/null | `let x = unwrap_or(val, 0);` |

## db_find_one vs db_find_one_result

### db_find_one (Simple)
Returns the document or null. Errors return error string.
```rhai
let doc = db_find_one("users", #{name: "Alice"});
if is_null(doc) {
    print("Not found");
} else if is_error(doc) {
    print("Error: " + get_error(doc));
} else {
    print("Found: " + doc.name);
}
```

### db_find_one_result (Explicit)
Returns a result object with explicit fields - preferred for clarity.
```rhai
let result = db_find_one_result("users", #{name: "Alice"});
if result.error != () {
    print("Error: " + result.error);
} else if !result.found {
    print("Not found");
} else {
    print("Found: " + result.doc.name);
}
```

## Utility Functions

| Function | Description |
|----------|-------------|
| `base64_encode(str)` | Encode string to base64 |
| `base64_decode(b64)` | Decode base64 to string |
| `print(msg)` | Log message (captured in result.logs) |
| `uuid()` | Generate UUID v4 string |
| `timestamp()` | Current Unix timestamp (seconds) |
| `timestamp_ms()` | Current Unix timestamp (milliseconds) |
| `now_iso()` | Current datetime as ISO 8601 string |

## Rhai Syntax Basics

### Variables & Types
```rhai
let x = 42;           // Integer
let s = "hello";      // String
let arr = [1, 2, 3];  // Array
let map = #{a: 1, b: 2};  // Object map (use #{ } for maps!)
```

### Query Operators in Maps
Use backticks for operators starting with `$`:
```rhai
let query = #{age: #{`$gt`: 18, `$lt`: 65}};
let docs = db_find("users", query);
```

### Control Flow
```rhai
// If/else
if x > 10 {
    print("big");
} else {
    print("small");
}

// For loop
for item in arr {
    print(item);
}

// While loop
while x > 0 {
    x -= 1;
}
```

### Functions
```rhai
fn greet(name) {
    "Hello, " + name + "!"
}

let msg = greet("World");
```

## Example Scripts

### Batch Insert with Logging
```rhai
let names = ["Alice", "Bob", "Carol"];
let inserted = 0;

for name in names {
    let doc = #{name: name, created: "2024-01-01"};
    let result = db_insert_one("users", doc);
    if !is_error(result) {
        inserted += 1;
        print("Inserted: " + name);
    }
}

#{inserted: inserted, total: names.len()}
```

### Safe Document Lookup
```rhai
let result = db_find_one_result("users", #{_id: params.user_id});

if result.error != () {
    #{success: false, error: result.error}
} else if !result.found {
    #{success: false, error: "User not found"}
} else {
    #{success: true, user: result.doc}
}
```

### Aggregation Report
```rhai
let pipeline = [
    #{`$match`: #{status: "completed"}},
    #{`$group`: #{
        _id: "$category",
        total: #{`$sum`: "$amount"},
        count: #{`$sum`: 1}
    }},
    #{`$sort`: #{total: -1}}
];

let results = db_aggregate("orders", pipeline);
if is_error(results) {
    #{error: get_error(results)}
} else {
    #{report: results, generated: "now"}
}
```

## Script Management

| Tool | Description |
|------|-------------|
| `script_save` | Save/update script with name, code, description |
| `script_list` | List all saved scripts |
| `script_get` | Get script code and metadata |
| `script_delete` | Delete a script |
| `script_history` | View version history |
| `script_rollback` | Restore previous version |
| `script_stats` | View execution statistics |

## Security & Limits

| Limit | Default | Description |
|-------|---------|-------------|
| Max operations | 1,000,000 | Prevents infinite loops (configurable via max_operations) |
| Max log entries | 10,000 | Prevents memory exhaustion |
| Max execution time | ~60s | Implicit via operation limit |

No file I/O or network access - scripts can only interact with the database."#
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

fn get_fuzzy_search_prompt(arguments: &Value) -> Value {
    let field = arguments
        .get("field")
        .and_then(|v| v.as_str())
        .unwrap_or("name");

    json!({
        "messages": [
            {
                "role": "user",
                "content": {
                    "type": "text",
                    "text": format!(r#"# IronBase Fuzzy Text Search Guide

Fuzzy search finds approximate string matches using similarity algorithms. Perfect for:
- Typo-tolerant search ("jonh" → "john")
- Name matching ("Jon" ≈ "John")
- Approximate text lookup

## Two Ways to Use Fuzzy Search

### Method 1: $fuzzy Query Operator (Simple)

Works on ANY field without index setup. Scans all documents.

**Simple form** (Jaro-Winkler, 0.8 threshold):
```json
// find tool
{{
  "collection": "users",
  "query": {{"{field}": {{"$fuzzy": "john"}}}}
}}
```

**Extended form** (custom algorithm and threshold):
```json
{{
  "collection": "users",
  "query": {{
    "{field}": {{
      "$fuzzy": {{
        "value": "john",
        "algorithm": "levenshtein",
        "threshold": 0.7
      }}
    }}
  }}
}}
```

### Method 2: Fuzzy Index + fuzzy_search Tool (Fast)

Pre-computed index for high-performance searches. Returns similarity scores.

**Step 1: Create fuzzy index**
```json
// index_create_fuzzy tool
{{
  "collection": "users",
  "field": "{field}",
  "algorithm": "jaro_winkler",
  "threshold": 0.8
}}
```

**Step 2: Search using index**
```json
// fuzzy_search tool
{{
  "collection": "users",
  "field": "{field}",
  "query": "john",
  "limit": 10
}}
```

**Response includes similarity scores:**
```json
[
  {{"_id": 1, "{field}": "John", "_similarity": 0.95}},
  {{"_id": 2, "{field}": "Jon", "_similarity": 0.87}},
  {{"_id": 3, "{field}": "Johnny", "_similarity": 0.82}}
]
```

## Algorithms Comparison

| Algorithm | Best For | Speed | Accuracy |
|-----------|----------|-------|----------|
| `jaro_winkler` | Names, short strings | ⚡ Fast | Good for prefix similarity |
| `levenshtein` | General text | Medium | Most accurate edit distance |
| `damerau_levenshtein` | Typos with transpositions | Medium | Handles "teh"→"the" |

### Algorithm Details

**Jaro-Winkler** (default)
- Gives higher scores to strings with matching prefixes
- "John" vs "Johnny" = 0.93 (high due to prefix match)
- "John" vs "nhoj" = 0.53 (low, no prefix match)
- Best for: First names, last names, short identifiers

**Levenshtein**
- Counts minimum edits (insert/delete/replace) needed
- "kitten" vs "sitting" = 0.57 (3 edits / 7 chars)
- "cat" vs "car" = 0.67 (1 edit / 3 chars)
- Best for: Spell checking, general string similarity

**Damerau-Levenshtein**
- Like Levenshtein but transpositions count as 1 edit
- "teh" vs "the" = 0.67 (Levenshtein) vs 0.67 (Damerau)
- "ab" vs "ba" = 0.0 (Levenshtein: 2 edits) vs 0.5 (Damerau: 1 transposition)
- Best for: User input with common typos

## Threshold Guidelines

| Threshold | Match Strictness | Example Use Case |
|-----------|------------------|------------------|
| 0.9+ | Very strict | Deduplication, exact-ish matches |
| 0.8 | Default | General name search |
| 0.7 | Lenient | Typo-tolerant search |
| 0.6 | Very lenient | Broad similarity matching |
| <0.5 | Not recommended | Too many false positives |

## Practical Examples

### 1. Name Search with Typo Tolerance
```json
// Find "Michael" even if typed as "Micheal" or "Michel"
{{
  "collection": "contacts",
  "query": {{"firstName": {{"$fuzzy": {{"value": "michael", "threshold": 0.75}}}}}}
}}
```

### 2. Product Search
```json
// Create index first
{{"collection": "products", "field": "title", "algorithm": "levenshtein", "threshold": 0.7}}

// Search
{{"collection": "products", "field": "title", "query": "iphone", "limit": 20}}
```

### 3. Email Domain Deduplication
```json
// Strict matching to find near-duplicates
{{
  "collection": "users",
  "query": {{"email": {{"$fuzzy": {{"value": "gmail.com", "threshold": 0.9}}}}}}
}}
```

### 4. Combining with Other Operators
```json
{{
  "collection": "users",
  "query": {{
    "$and": [
      {{"city": "NYC"}},
      {{"{field}": {{"$fuzzy": "smith"}}}}
    ]
  }}
}}
```

## Performance Considerations

| Approach | Use When | Performance |
|----------|----------|-------------|
| `$fuzzy` operator | Ad-hoc queries, small collections | O(n) scan |
| `fuzzy_search` tool | Repeated searches, large collections | O(1) index lookup |

**Index tradeoffs:**
- ✅ Fast searches with similarity scores
- ✅ Results pre-sorted by relevance
- ❌ Index build time on insert/update
- ❌ Additional storage space

## Best Practices

1. **Choose algorithm by use case**: Jaro-Winkler for names, Levenshtein for text
2. **Start with threshold 0.8**: Adjust based on false positive/negative rate
3. **Use indexes for production**: `$fuzzy` is fine for development
4. **Combine with exact matches**: Use `$and` to filter first, then fuzzy
5. **Limit results**: Fuzzy matches can return many documents

## Limitations

- Case-sensitive by default (normalize before storing)
- Works only on string fields
- No stemming or language-aware matching
- Index must match search algorithm for best results"#, field = field)
                }
            }
        ]
    })
}

fn get_fulltext_search_prompt(arguments: &Value) -> Value {
    let language = arguments
        .get("language")
        .and_then(|v| v.as_str())
        .unwrap_or("hungarian");

    json!({
        "messages": [
            {
                "role": "user",
                "content": {
                    "type": "text",
                    "text": format!(r#"# Full-Text Search Guide

IronBase provides TF-IDF based full-text search with language-aware stemming and stop word removal.

## Supported Languages

| Language | Stemming | Stop Words | Code |
|----------|----------|------------|------|
| Hungarian | ✅ Snowball | ✅ 40+ words | `hungarian` |
| English | ✅ Snowball | ✅ 100+ words | `english` |
| German | ✅ Snowball | ✅ 80+ words | `german` |
| None | ❌ | ❌ | `none` |

## Step 1: Create Full-Text Index

```json
// index_create_fulltext tool
{{
  "collection": "articles",
  "field": "content",
  "language": "{language}"
}}
```

Optional parameters:
- `min_word_length`: Minimum word length to index (default: 2)
- `accent_folding`: Convert áéíóú → aeiou (default: true)

## Step 2: Search Documents

```json
// fulltext_search tool
{{
  "collection": "articles",
  "field": "content",
  "query": "database optimization",
  "limit": 10
}}
```

### Response Format

```json
{{
  "results": [
    {{
      "document": {{"_id": "...", "title": "...", "content": "..."}},
      "score": 2.847,
      "matched_tokens": ["databas", "optim"]
    }}
  ],
  "total_matches": 42
}}
```

## TF-IDF Scoring Explained

**TF (Term Frequency)**: How often the term appears in the document
**IDF (Inverse Document Frequency)**: How rare the term is across all documents

```
Score = TF × IDF = (term_count / doc_length) × log(total_docs / docs_with_term)
```

Higher scores = more relevant documents

## Advanced Options

### Pagination
```json
{{
  "collection": "articles",
  "field": "content",
  "query": "király",
  "limit": 10,
  "skip": 20
}}
```

### Minimum Score Threshold
```json
{{
  "collection": "articles",
  "field": "content",
  "query": "király",
  "min_score": 0.5,
  "limit": 10
}}
```

### Projection (Reduce Response Size)
```json
{{
  "collection": "articles",
  "field": "content",
  "query": "király",
  "limit": 10,
  "projection": {{"title": 1, "_id": 1}}
}}
```

Exclude large fields:
```json
{{
  "projection": {{"full_text": 0, "raw_html": 0}}
}}
```

## Language-Specific Examples

### Hungarian
```json
// Index
{{"collection": "cikkek", "field": "tartalom", "language": "hungarian"}}

// Search - finds: király, királyok, királyt, királynak
{{"collection": "cikkek", "field": "tartalom", "query": "király", "limit": 10}}
```

### English
```json
// Index
{{"collection": "articles", "field": "body", "language": "english"}}

// Search - finds: running, runs, ran (all stemmed to "run")
{{"collection": "articles", "field": "body", "query": "running", "limit": 10}}
```

## Stemming Examples

| Language | Input | Stem |
|----------|-------|------|
| Hungarian | királyok | király |
| Hungarian | futottam | fut |
| English | running | run |
| English | optimization | optim |
| German | Häuser | haus |

## Fulltext vs Fuzzy Search

| Feature | Fulltext Search | Fuzzy Search |
|---------|-----------------|--------------|
| Algorithm | TF-IDF | Similarity (Jaro-Winkler, etc.) |
| Use case | Document search, articles | Typo tolerance, names |
| Stemming | ✅ Yes | ❌ No |
| Stop words | ✅ Filtered | ❌ No |
| Multi-word | ✅ Yes | ❌ Single field |
| Scoring | Relevance-based | Similarity 0.0-1.0 |

## Best Practices

1. **Choose the right language**: Affects stemming and stop words
2. **Use projection**: Large documents can overflow context
3. **Set reasonable limits**: Start with 10-20 results
4. **Use min_score**: Filter out low-relevance matches
5. **Index relevant fields only**: Don't index IDs or dates

## Rhai Scripting

```rhai
// Create index
db_create_fulltext_index("articles", "content", "{language}");

// Search
let results = db_fulltext_search("articles", "content", "query", 10);
for r in results {{
    print(r.document.title + " (score: " + r.score + ")");
}}
```

## Limitations

- One fulltext index per field per collection
- Query must be at least `min_word_length` characters
- Stop words are removed from queries
- Cannot combine with regular queries (use separate searches)"#, language = language)
                }
            }
        ]
    })
}

fn get_acl_guide_prompt() -> Value {
    json!({
        "messages": [
            {
                "role": "user",
                "content": {
                    "type": "text",
                    "text": r#"# Access Control List (ACL) Guide

IronBase provides collection-level permissions based on client origin.

## Interface Types

| Type | Description | Example IPs |
|------|-------------|-------------|
| `localhost` | Loopback address | 127.0.0.1, ::1 |
| `internal` | Private network (RFC 1918) | 10.x.x.x, 172.16-31.x.x, 192.168.x.x |
| `external` | Public internet | Everything else |

## Permission Levels

| Permission | Includes | Operations |
|------------|----------|------------|
| `read` | - | find, count, aggregate, explain |
| `write` | read | insert, update, delete |
| `admin` | read, write | create/drop index, schema, compact |
| `all` | everything | All operations |

## Default Permissions (No ACL Set)

| Interface | Permissions |
|-----------|-------------|
| localhost | all |
| internal | read, write |
| external | read |

## Setting ACL Rules

```json
// acl_set tool
{
  "collection": "users",
  "rules": [
    {"principal": "interface:localhost", "permissions": "all"},
    {"principal": "interface:internal", "permissions": "read,write"},
    {"principal": "interface:external", "permissions": "read"}
  ]
}
```

## Principal Types

| Principal | Description | Example |
|-----------|-------------|---------|
| `interface:localhost` | Loopback connections | Local development |
| `interface:internal` | Private network | Backend services |
| `interface:external` | Public internet | Public API |
| `apikey:sk-xxx` | Specific API key | Privileged service |
| `ip:192.168.1.100` | Exact IP address | Specific server |
| `iprange:10.0.0.0/8` | CIDR range | Subnet |
| `anyone` | All clients | Public data |

## ACL Tools

### List All ACLs
```json
// acl_list tool
{}
```

### Get ACL for Collection
```json
// acl_get tool
{"collection": "users"}
```

### Set ACL
```json
// acl_set tool (localhost only)
{
  "collection": "users",
  "rules": [
    {"principal": "interface:localhost", "permissions": "all"},
    {"principal": "apikey:sk-admin-key", "permissions": "admin"},
    {"principal": "interface:internal", "permissions": "read,write"},
    {"principal": "interface:external", "permissions": "read"}
  ]
}
```

### Delete ACL (Revert to Defaults)
```json
// acl_delete tool (localhost only)
{"collection": "users"}
```

### Cleanup Orphaned ACLs
```json
// acl_cleanup tool (localhost only)
{}
```

## Example Scenarios

### 1. Public Read-Only API
```json
{
  "collection": "products",
  "rules": [
    {"principal": "interface:localhost", "permissions": "all"},
    {"principal": "anyone", "permissions": "read"}
  ]
}
```

### 2. Internal Service Write Access
```json
{
  "collection": "orders",
  "rules": [
    {"principal": "interface:localhost", "permissions": "all"},
    {"principal": "interface:internal", "permissions": "read,write"},
    {"principal": "interface:external", "permissions": ""}
  ]
}
```

### 3. API Key Based Access
```json
{
  "collection": "sensitive_data",
  "rules": [
    {"principal": "interface:localhost", "permissions": "all"},
    {"principal": "apikey:sk-trusted-service", "permissions": "read,write"},
    {"principal": "anyone", "permissions": ""}
  ]
}
```

### 4. IP Whitelist
```json
{
  "collection": "admin_logs",
  "rules": [
    {"principal": "interface:localhost", "permissions": "all"},
    {"principal": "ip:10.0.1.50", "permissions": "read"},
    {"principal": "iprange:10.0.2.0/24", "permissions": "read"}
  ]
}
```

## System Collections

System collections (`_system.*`) have special protection:

| Collection | Read Access | Write Access |
|------------|-------------|--------------|
| `_system.scripts` | localhost, internal, external | localhost only |
| `_system.api_keys` | localhost only | localhost only |
| `_system.acl` | localhost only | localhost only |
| `_system.listeners` | localhost only | localhost only |

## Rule Evaluation Order

1. Check if collection has explicit ACL rules
2. Find matching principal (most specific first):
   - `ip:exact` > `iprange:cidr` > `apikey:xxx` > `interface:xxx` > `anyone`
3. If no match found, use default permissions for interface type
4. Deny if no permission grants the operation

## Best Practices

1. **Always set localhost to `all`**: You need admin access from local machine
2. **Use API keys for services**: More granular than IP-based rules
3. **Deny by default for sensitive data**: Empty permissions = deny all
4. **Cleanup orphaned ACLs**: Run `acl_cleanup` after dropping collections
5. **Test from different interfaces**: Verify rules work as expected"#
                }
            }
        ]
    })
}

fn get_database_admin_prompt() -> Value {
    json!({
        "messages": [
            {
                "role": "user",
                "content": {
                    "type": "text",
                    "text": r#"# Database Administration Guide

## Database Statistics

```json
// db_stats tool
{}
```

Returns:
```json
{
  "version": "1.0.68",
  "collections": 5,
  "total_documents": 12345,
  "database_size_bytes": 5242880,
  "uptime_seconds": 3600,
  "collections_detail": {
    "users": {"documents": 1000, "indexes": 3},
    "orders": {"documents": 5000, "indexes": 2}
  }
}
```

## Database Compaction

Removes tombstones and reclaims disk space:

```json
// db_compact tool
{"collection": "users"}
```

When to compact:
- After bulk deletes
- When database file is much larger than data
- During maintenance windows

## Database Checkpoint

Force WAL flush to disk:

```json
// db_checkpoint tool
{}
```

Use cases:
- Before backup
- After important batch operations
- Before server shutdown

## Collection Management

### List Collections
```json
// collection_list tool
{}
```

### Create Collection
```json
// insert_one tool (auto-creates collection)
{"collection": "new_collection", "document": {"_id": "init"}}
```

### Drop Collection
```json
// collection_drop tool
{"collection": "old_collection"}
```

### Get Collection Stats
```json
// count_documents tool
{"collection": "users", "filter": {}}
```

## Index Management

### List Indexes
```json
// index_list tool
{"collection": "users"}
```

### Create Index
```json
// index_create tool
{"collection": "users", "field": "email", "unique": true}
```

### Create Compound Index
```json
// index_create_compound tool
{"collection": "orders", "fields": ["user_id", "created_at"], "unique": false}
```

### Drop Index
```json
// index_drop tool
{"collection": "users", "index_name": "email_1"}
```

## Query Analysis

### Explain Query Plan
```json
// explain tool
{
  "collection": "users",
  "query": {"email": "test@example.com"}
}
```

Returns:
```json
{
  "plan": "IndexScan",
  "index_used": "email_1",
  "estimated_docs": 1,
  "query_time_ms": 0.5
}
```

## Durability Modes

| Mode | Description | Use Case |
|------|-------------|----------|
| Safe | fsync after each write | Production (default) |
| Batch | fsync after N writes | Bulk imports |
| Unsafe | No auto-fsync | Testing only |

## Backup Operations

### Hot Backup (Lock-Free)

IronBase supports hot backup without stopping the server:

```bash
# Full backup
ironbase-backup backup --db /path/to/data.mlite --output ./backups --full

# Incremental backup
ironbase-backup backup --db /path/to/data.mlite --output ./backups

# Restore
ironbase-backup restore --backup ./backups/backup_xxx.tar.zst --output /path/to/restored.mlite
```

### Why Lock-Free Works

IronBase uses append-only storage:
- Documents are never modified in-place
- Updates create new versions + tombstones
- All data up to `data_end_offset` is immutable
- Backup reads immutable data safely

## Performance Tuning

### Monitor Slow Queries
1. Use `explain` tool to check query plans
2. Create indexes for frequently queried fields
3. Use projections to reduce data transfer

### Optimize Storage
1. Run compaction after bulk deletes
2. Use appropriate data types (avoid storing large blobs)
3. Consider sharding for very large datasets

### Memory Usage
- Query cache: ~1000 entries (LRU eviction)
- Index cache: In-memory B+ trees
- Document cache: None (disk-based)

## Maintenance Checklist

### Daily
- [ ] Check `db_stats` for anomalies
- [ ] Monitor query response times

### Weekly
- [ ] Review slow query logs
- [ ] Check index usage with `explain`

### Monthly
- [ ] Run `db_compact` on high-churn collections
- [ ] Verify backup restoration
- [ ] Review and cleanup unused indexes

## Troubleshooting

### Database Won't Open
1. Check file permissions
2. Verify no other process holds the lock
3. Check WAL for corruption

### Slow Queries
1. Run `explain` to check plan
2. Create appropriate indexes
3. Use projections and limits

### High Disk Usage
1. Run `db_compact` on collections
2. Check for orphaned collections
3. Review document sizes"#
                }
            }
        ]
    })
}

fn get_security_guide_prompt() -> Value {
    json!({
        "messages": [
            {
                "role": "user",
                "content": {
                    "type": "text",
                    "text": r#"# Security Guide

Comprehensive guide for securing your IronBase deployment.

## Security Layers

```
┌─────────────────────────────────────────┐
│           TLS/HTTPS (Encryption)         │
├─────────────────────────────────────────┤
│         API Key Authentication           │
├─────────────────────────────────────────┤
│      ACL (Collection Permissions)        │
├─────────────────────────────────────────┤
│     Interface Type (localhost/etc)       │
└─────────────────────────────────────────┘
```

## 1. API Key Authentication

### Enable API Keys
Set in config.toml:
```toml
[security]
require_api_key = true
api_key_cache_ttl = 60
```

Set admin key via environment:
```bash
export IRONBASE_ADMIN_KEY="your-secure-admin-key"
```

### Create API Key
```json
// admin_apikey_create tool (requires admin_key)
{
  "admin_key": "your-admin-key",
  "name": "backend-service"
}
```

Response:
```json
{
  "id": "key_abc123",
  "key": "sk-xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx",
  "name": "backend-service",
  "created_at": "2024-01-15T10:30:00Z"
}
```

**⚠️ IMPORTANT**: Save the `key` value! It's only shown once.

### List API Keys
```json
// admin_apikey_list tool
{"admin_key": "your-admin-key"}
```

Returns masked keys:
```json
[
  {"id": "key_abc123", "name": "backend-service", "key_preview": "sk-xxxx...xxxx", "enabled": true}
]
```

### Revoke/Delete API Key
```json
// Disable (can re-enable)
{"admin_key": "your-admin-key", "id": "key_abc123"}

// Delete permanently
{"admin_key": "your-admin-key", "id": "key_abc123"}
```

### Using API Keys

Via HTTP Header (recommended):
```bash
curl -X POST https://server:8080/mcp \
    -H "Authorization: Bearer sk-your-api-key" \
    -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":1,"method":"tools/call",...}'
```

Via JSON parameter:
```json
{
  "name": "find",
  "arguments": {
    "api_key": "sk-your-api-key",
    "collection": "users",
    "query": {}
  }
}
```

## 2. TLS/HTTPS Configuration

### Generate Certificates
```bash
# Self-signed (development)
openssl req -x509 -newkey rsa:4096 -keyout key.pem -out cert.pem -days 365 -nodes

# Let's Encrypt (production)
certbot certonly --standalone -d your-domain.com
```

### Enable TLS in config.toml
```toml
[tls]
enabled = true
cert_file = "/path/to/cert.pem"
key_file = "/path/to/key.pem"
```

### Multiple Listeners with TLS
```json
// Add HTTPS listener
{
  "id": "external-https",
  "bind": "0.0.0.0:443",
  "tls": true,
  "cert_path": "/etc/ssl/certs/server.crt",
  "key_path": "/etc/ssl/private/server.key"
}

// Add HTTP listener (internal only)
{
  "id": "internal-http",
  "bind": "192.168.1.100:8080",
  "tls": false
}
```

## 3. ACL (Access Control Lists)

See `acl-guide` prompt for detailed ACL configuration.

Quick example:
```json
{
  "collection": "sensitive_data",
  "rules": [
    {"principal": "interface:localhost", "permissions": "all"},
    {"principal": "apikey:sk-trusted", "permissions": "read,write"},
    {"principal": "interface:internal", "permissions": "read"},
    {"principal": "interface:external", "permissions": ""}
  ]
}
```

## 4. Network Security

### Firewall Rules
```bash
# Allow only internal network
iptables -A INPUT -p tcp --dport 8080 -s 10.0.0.0/8 -j ACCEPT
iptables -A INPUT -p tcp --dport 8080 -j DROP

# Or use UFW
ufw allow from 10.0.0.0/8 to any port 8080
```

### Bind to Specific Interface
```toml
[server]
host = "192.168.1.100"  # Don't use 0.0.0.0 unless needed
port = 8080
```

## Security Checklist

### Deployment
- [ ] Enable TLS for all external connections
- [ ] Set strong `IRONBASE_ADMIN_KEY`
- [ ] Enable `require_api_key` in production
- [ ] Bind to internal interfaces only
- [ ] Configure firewall rules

### API Keys
- [ ] Create separate keys for each service
- [ ] Use meaningful names for tracking
- [ ] Rotate keys periodically
- [ ] Revoke unused keys
- [ ] Store keys securely (vault, env vars)

### ACL
- [ ] Set explicit ACLs on sensitive collections
- [ ] Use `interface:external` with minimal permissions
- [ ] Prefer API key principals over IP-based rules
- [ ] Run `acl_cleanup` periodically

### Monitoring
- [ ] Log authentication failures
- [ ] Monitor for unusual access patterns
- [ ] Set up alerts for admin operations

## Common Security Mistakes

### ❌ Exposing Without Auth
```toml
# WRONG - No authentication on public interface
[server]
host = "0.0.0.0"
[security]
require_api_key = false
```

### ✅ Secure Configuration
```toml
[server]
host = "0.0.0.0"
[security]
require_api_key = true
[tls]
enabled = true
cert_file = "/path/to/cert.pem"
key_file = "/path/to/key.pem"
```

### ❌ Weak Admin Key
```bash
export IRONBASE_ADMIN_KEY="admin"  # WRONG
```

### ✅ Strong Admin Key
```bash
export IRONBASE_ADMIN_KEY="$(openssl rand -hex 32)"  # RIGHT
```

## Security Best Practices

1. **Defense in depth**: Use all security layers
2. **Least privilege**: Grant minimum required permissions
3. **Secure by default**: Enable all security features
4. **Audit regularly**: Review logs and access patterns
5. **Keep updated**: Apply security patches promptly"#
                }
            }
        ]
    })
}

fn get_listener_config_prompt() -> Value {
    json!({
        "messages": [
            {
                "role": "user",
                "content": {
                    "type": "text",
                    "text": r#"# Listener Configuration Guide

Configure multiple HTTP/HTTPS endpoints for your IronBase MCP server.

## Why Multiple Listeners?

- Separate internal (HTTP) and external (HTTPS) interfaces
- Different ports for different services
- Interface-specific TLS configurations
- ACL rules based on interface type

## Listener Tools

### List All Listeners
```json
// listener_list tool
{}
```

### Get Listener Details
```json
// listener_get tool
{"id": "internal"}
```

### Add HTTP Listener
```json
// listener_add tool
{
  "id": "internal",
  "bind": "192.168.1.100:8080",
  "tls": false,
  "description": "Internal API endpoint"
}
```

### Add HTTPS Listener
```json
// listener_add tool
{
  "id": "external",
  "bind": "0.0.0.0:443",
  "tls": true,
  "cert_path": "/etc/ssl/certs/server.crt",
  "key_path": "/etc/ssl/private/server.key",
  "description": "Public HTTPS endpoint"
}
```

### Enable/Disable Listener
```json
// listener_enable tool
{"id": "external"}

// listener_disable tool
{"id": "internal"}
```

### Delete Listener
```json
// listener_delete tool
{"id": "old-listener"}
```

## Listener Configuration Fields

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `id` | string | Yes | Unique identifier |
| `bind` | string | Yes | Address:port (e.g., "0.0.0.0:8080") |
| `tls` | bool | No | Enable HTTPS (default: false) |
| `cert_path` | string | If tls=true | Path to TLS certificate |
| `key_path` | string | If tls=true | Path to TLS private key |
| `enabled` | bool | No | Active status (default: true) |
| `description` | string | No | Human-readable description |

## Common Configurations

### 1. Development Setup
```json
// Single HTTP listener on localhost
{
  "id": "dev",
  "bind": "127.0.0.1:8080",
  "tls": false,
  "description": "Development server"
}
```

### 2. Production with Separate Interfaces
```json
// Internal HTTP (for backend services)
{
  "id": "internal",
  "bind": "10.0.0.50:8080",
  "tls": false,
  "description": "Internal service API"
}

// External HTTPS (for public access)
{
  "id": "external",
  "bind": "0.0.0.0:443",
  "tls": true,
  "cert_path": "/etc/letsencrypt/live/example.com/fullchain.pem",
  "key_path": "/etc/letsencrypt/live/example.com/privkey.pem",
  "description": "Public API endpoint"
}
```

### 3. Microservices Architecture
```json
// Service A endpoint
{
  "id": "service-a",
  "bind": "10.0.1.10:8080",
  "tls": false,
  "description": "Service A database endpoint"
}

// Service B endpoint
{
  "id": "service-b",
  "bind": "10.0.1.20:8080",
  "tls": false,
  "description": "Service B database endpoint"
}

// Admin endpoint (localhost only)
{
  "id": "admin",
  "bind": "127.0.0.1:9090",
  "tls": false,
  "description": "Admin/maintenance endpoint"
}
```

## Integration with ACL

Listeners determine the interface type for ACL:

| Bind Address | Interface Type |
|--------------|----------------|
| 127.0.0.1:* | localhost |
| 10.*.*.* | internal |
| 172.16-31.*.* | internal |
| 192.168.*.* | internal |
| 0.0.0.0:* | depends on client IP |

### Example: Different Permissions per Interface
```json
// ACL for users collection
{
  "collection": "users",
  "rules": [
    {"principal": "interface:localhost", "permissions": "all"},
    {"principal": "interface:internal", "permissions": "read,write"},
    {"principal": "interface:external", "permissions": "read"}
  ]
}
```

## TLS Certificate Setup

### Self-Signed (Development)
```bash
openssl req -x509 -newkey rsa:4096 \
    -keyout key.pem -out cert.pem \
    -days 365 -nodes \
  -subj "/CN=localhost"
```

### Let's Encrypt (Production)
```bash
# Install certbot
apt install certbot

# Get certificate
certbot certonly --standalone -d your-domain.com

# Certificate paths
# /etc/letsencrypt/live/your-domain.com/fullchain.pem
# /etc/letsencrypt/live/your-domain.com/privkey.pem
```

### Certificate Renewal
```bash
# Auto-renewal with certbot
certbot renew --quiet

# Reload server after renewal
systemctl reload ironbase-mcp
```

## Storage

Listener configurations are stored in `_system.listeners` collection:
- Localhost-only write access
- Persisted across server restarts
- Can be managed via listener_* tools

## Best Practices

1. **Separate internal/external**: Use different listeners for different trust levels
2. **Always TLS for external**: Never expose HTTP to the internet
3. **Bind to specific IPs**: Avoid 0.0.0.0 unless necessary
4. **Use meaningful IDs**: "internal-api" not "listener1"
5. **Document with descriptions**: Helps with maintenance
6. **Disable unused listeners**: Don't leave test listeners active

## Troubleshooting

### Port Already in Use
```bash
# Find process using port
lsof -i :8080
netstat -tlnp | grep 8080

# Kill if needed
kill -9 <PID>
```

### Certificate Errors
```bash
# Verify certificate
openssl x509 -in cert.pem -text -noout

# Check key matches cert
openssl x509 -noout -modulus -in cert.pem | md5sum
openssl rsa -noout -modulus -in key.pem | md5sum
# Should match!
```

### Listener Not Starting
1. Check bind address is valid
2. Verify port is available
3. Ensure cert/key paths are correct
4. Check file permissions on certificates"#
                }
            }
        ]
    })
}
