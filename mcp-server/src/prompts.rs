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
