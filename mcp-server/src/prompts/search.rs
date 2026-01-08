//! Search and optimization prompts
//!
//! Contains: schema-validation, index-optimization, fuzzy-search, fulltext-search

use serde_json::{json, Value};

pub fn schema_validation(arguments: &Value) -> Value {
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
- `schema_set`: Set/update schema (use `null` to remove validation)
- `schema_get`: View current schema"#, collection, collection)
                }
            }
        ]
    })
}

pub fn index_optimization(arguments: &Value) -> Value {
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
// index_create tool
{{"collection": "{}", "field": "email", "unique": true}}
```

### Compound Index (multiple fields)
```json
// index_create tool
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

Use the `explain` tool to see query plan:
```json
// explain tool
{{"collection": "{}", "query": {{"status": "active"}}}}
```

## Index Hints

Force specific index usage:
```json
// find_with_hint tool
{{"collection": "{}", "query": {{}}, "hint": "status_1"}}
```

## Listing Indexes
```json
// index_list tool
{{"collection": "{}"}}
```

## Dropping Indexes
```json
// index_drop tool
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
- `$regex` without anchor (^) cannot use index

## Index-Based Aggregation (2300x speedup!)

When a `$group` pipeline meets these conditions, IronBase reads ONLY the index:

1. **NO leading `$match`** stage
2. Single field group key: `{{"_id": "$field"}}`
3. All accumulators are `$sum: 1` (counting)
4. Single-field index exists on the group field

### Example Performance (78K docs, 39GB database):
```json
// FAST (47ms) - Uses index, no document loading
[{{"$group": {{"_id": "$email", "count": {{"$sum": 1}}}}}}, {{"$sort": {{"count": -1}}}}, {{"$limit": 5}}]

// SLOW (284s) - $match disables index optimization, loads all docs
[{{"$match": {{"email": {{"$exists": true}}}}}}, {{"$group": {{"_id": "$email", "count": {{"$sum": 1}}}}}}]
```

**Tip:** For full collection counts, skip the `$match` stage entirely!"#,
                        collection, collection, collection, collection, collection, collection, collection)
                }
            }
        ]
    })
}

pub fn fuzzy_search(arguments: &Value) -> Value {
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
{{
  "results": [
    {{"document": {{"_id": 1, "{field}": "John"}}, "score": 0.95}},
    {{"document": {{"_id": 2, "{field}": "Jon"}}, "score": 0.87}},
    {{"document": {{"_id": 3, "{field}": "Johnny"}}, "score": 0.82}}
  ],
  "count": 3
}}
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

pub fn fulltext_search(arguments: &Value) -> Value {
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
  "count": 1
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
let results = db_fulltext_search("articles", "content", "query", #{{limit: 10}});
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
