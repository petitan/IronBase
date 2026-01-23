//! Server instructions for LLM clients

/// Get server instructions for LLM clients (sent in initialize response)
/// These instructions help Claude Desktop and other MCP clients generate better queries
pub(crate) fn get_server_instructions() -> String {
    r#"# IronBase MCP Server

IronBase is a high-performance embedded NoSQL document database with MongoDB-compatible query syntax. Single-file storage (.mlite), zero configuration.

## Core Capabilities
- **68 tools**: CRUD, aggregation pipelines, full-text search, fuzzy search, indexes, transactions, scripting
- **Query operators**: $eq, $ne, $gt, $gte, $lt, $lte, $in, $nin, $and, $or, $not, $regex, $exists, $elemMatch, $all, $size
- **Update operators**: $set, $inc, $unset, $push, $pull, $addToSet, $pop
- **Aggregation stages**: $match, $group, $project, $sort, $limit, $skip, $unwind, $count
- **Accumulators**: $sum, $avg, $min, $max, $first, $last
- **Full-text search**: TF-IDF scoring with Hungarian/English/German stemming
- **Fuzzy search**: Jaro-Winkler, Levenshtein, Damerau-Levenshtein algorithms
- **B+ tree indexes**: single-field, compound, unique, sparse

## Essential Rules

### 1. Always use LIMIT
- `find`: default is 10,000 - always specify smaller (10-100)
- `aggregate`: always end with `{"$limit": N}`

### 2. Check size before fetching
Use `count_documents` before `find` on unknown collections.

### 3. Use projection to reduce response size
Only request needed fields: `"projection": {"name": 1, "email": 1}`
Exclude large fields: `"projection": {"body": 0, "content": 0}`

### 4. Date range queries - use comparison operators
✅ FAST: `{"date": {"$gte": "2024-01-01", "$lt": "2025-01-01"}}` (uses index)
❌ SLOW: `{"date": {"$regex": "^2024"}}` (collection scan)

### 5. Aggregation - filter first with $match
✅ FAST: `[{"$match": {"status": "active"}}, {"$group": {...}}, {"$limit": 10}]`
❌ SLOW: `[{"$group": {...}}]` (scans all documents)

## Key Tools

| Tool | Purpose |
|------|---------|
| `find` | Query documents with filter, projection, sort, limit, skip |
| `find_one` | Get single document |
| `count_documents` | Count matching documents |
| `aggregate` | Run aggregation pipeline |
| `fulltext_search` | TF-IDF text search (requires fulltext index) |
| `fuzzy_search` | Approximate string matching (requires fuzzy index) |
| `explain` | Analyze query execution plan |
| `index_list` | Show collection indexes |

## Scripting (Rhai)
Use `script_exec` for complex operations:
```
db_find("collection", #{query}, #{limit: 10})
db_count("collection", #{status: "active"})
db_aggregate("collection", [#{...}])
```
Note: Use `db_find()` NOT `db.find()` - JavaScript style is not supported."#.to_string()
}
