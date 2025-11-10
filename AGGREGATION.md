# Aggregation Pipeline Documentation

MongoLite supports MongoDB-compatible aggregation pipelines for advanced data processing and analysis.

## Table of Contents

- [Overview](#overview)
- [Quick Start](#quick-start)
- [Pipeline Stages](#pipeline-stages)
  - [$match](#match---filter-documents)
  - [$group](#group---group-documents)
  - [$project](#project---reshape-documents)
  - [$sort](#sort---sort-documents)
  - [$limit](#limit---limit-results)
  - [$skip](#skip---skip-documents)
- [Accumulator Operators](#accumulator-operators)
- [Real-World Examples](#real-world-examples)

---

## Overview

An aggregation pipeline consists of one or more **stages** that process documents sequentially. Each stage transforms the documents and passes them to the next stage.

```python
results = collection.aggregate([
    {"$match": {...}},      # Stage 1: Filter
    {"$group": {...}},      # Stage 2: Group
    {"$sort": {...}},       # Stage 3: Sort
    {"$limit": 10}          # Stage 4: Limit
])
```

---

## Quick Start

```python
import ironbase

# Open database
db = ironbase.MongoLite("mydb.db")
users = db.collection("users")

# Insert sample data
users.insert_one({"name": "Alice", "age": 30, "city": "NYC"})
users.insert_one({"name": "Bob", "age": 25, "city": "LA"})
users.insert_one({"name": "Carol", "age": 35, "city": "NYC"})

# Simple aggregation: Count users by city
results = users.aggregate([
    {"$group": {"_id": "$city", "count": {"$sum": 1}}}
])

# Output: [{"_id": "NYC", "count": 2}, {"_id": "LA", "count": 1}]
```

---

## Pipeline Stages

### $match - Filter Documents

Filters documents based on query conditions. Similar to `find()`.

**Syntax:**
```python
{"$match": {<query>}}
```

**Example 1: Filter by age**
```python
results = users.aggregate([
    {"$match": {"age": {"$gte": 30}}}
])
# Returns users aged 30 or older
```

**Example 2: Multiple conditions**
```python
results = users.aggregate([
    {"$match": {
        "age": {"$gte": 25, "$lt": 40},
        "city": "NYC"
    }}
])
# Returns NYC users aged 25-39
```

**Query Operators Supported:**
- `$eq`, `$ne`, `$gt`, `$gte`, `$lt`, `$lte`
- `$in`, `$nin`
- `$and`, `$or`, `$not`
- `$exists`, `$type`
- `$regex`

---

### $group - Group Documents

Groups documents by a specified field and computes aggregated values.

**Syntax:**
```python
{"$group": {
    "_id": <expression>,        # Group by field
    "<field>": {<accumulator>}  # Computed field
}}
```

**Example 1: Count by city**
```python
results = users.aggregate([
    {"$group": {
        "_id": "$city",
        "count": {"$sum": 1}
    }}
])
# Output: [{"_id": "NYC", "count": 2}, {"_id": "LA", "count": 1}]
```

**Example 2: Multiple accumulators**
```python
results = users.aggregate([
    {"$group": {
        "_id": "$city",
        "totalUsers": {"$sum": 1},
        "avgAge": {"$avg": "$age"},
        "minAge": {"$min": "$age"},
        "maxAge": {"$max": "$age"}
    }}
])
# Output: [{"_id": "NYC", "totalUsers": 2, "avgAge": 32.5, "minAge": 30, "maxAge": 35}, ...]
```

**Example 3: Group all documents (_id: null)**
```python
results = users.aggregate([
    {"$group": {
        "_id": None,  # or null in JSON
        "total": {"$sum": 1},
        "avgAge": {"$avg": "$age"}
    }}
])
# Output: [{"_id": null, "total": 3, "avgAge": 30.0}]
```

**Example 4: $first and $last**
```python
# Get first and last user in each city (when sorted by age)
results = users.aggregate([
    {"$sort": {"age": 1}},
    {"$group": {
        "_id": "$city",
        "youngest": {"$first": "$name"},
        "oldest": {"$last": "$name"}
    }}
])
```

---

### $project - Reshape Documents

Includes, excludes, or renames fields in documents.

**Syntax:**
```python
{"$project": {
    "<field>": 1,     # Include field
    "<field>": 0,     # Exclude field
    "_id": 0          # Exclude _id (special case)
}}
```

**Example 1: Include specific fields**
```python
results = users.aggregate([
    {"$project": {
        "name": 1,
        "city": 1,
        "_id": 0  # Exclude _id
    }}
])
# Output: [{"name": "Alice", "city": "NYC"}, ...]
```

**Example 2: Exclude fields**
```python
results = users.aggregate([
    {"$project": {
        "password": 0,  # Exclude password field
        "ssn": 0        # Exclude SSN field
    }}
])
# Returns all fields except password and ssn
```

**Important Notes:**
- **Include mode**: `{"field": 1}` - only specified fields are included
- **Exclude mode**: `{"field": 0}` - all fields except specified are included
- **Cannot mix** include and exclude (except for `_id`)
- Excluding `_id` is allowed in include mode: `{"name": 1, "_id": 0}`

---

### $sort - Sort Documents

Sorts documents by one or more fields.

**Syntax:**
```python
{"$sort": {
    "<field>": 1,   # Ascending
    "<field>": -1   # Descending
}}
```

**Example 1: Sort by single field**
```python
results = users.aggregate([
    {"$sort": {"age": -1}}  # Descending by age
])
# Oldest users first
```

**Example 2: Sort by multiple fields**
```python
results = users.aggregate([
    {"$sort": {
        "city": 1,   # City ascending
        "age": -1    # Age descending within city
    }}
])
```

**Example 3: Sort after grouping**
```python
results = users.aggregate([
    {"$group": {"_id": "$city", "count": {"$sum": 1}}},
    {"$sort": {"count": -1}}  # Cities with most users first
])
```

---

### $limit - Limit Results

Limits the number of documents returned.

**Syntax:**
```python
{"$limit": <number>}
```

**Example 1: Top 10 results**
```python
results = users.aggregate([
    {"$sort": {"age": -1}},
    {"$limit": 10}
])
# Top 10 oldest users
```

**Example 2: Top 3 cities**
```python
results = users.aggregate([
    {"$group": {"_id": "$city", "count": {"$sum": 1}}},
    {"$sort": {"count": -1}},
    {"$limit": 3}
])
```

---

### $skip - Skip Documents

Skips a specified number of documents.

**Syntax:**
```python
{"$skip": <number>}
```

**Example: Pagination**
```python
# Page 1 (results 1-10)
page1 = users.aggregate([
    {"$sort": {"name": 1}},
    {"$limit": 10}
])

# Page 2 (results 11-20)
page2 = users.aggregate([
    {"$sort": {"name": 1}},
    {"$skip": 10},
    {"$limit": 10}
])

# Page 3 (results 21-30)
page3 = users.aggregate([
    {"$sort": {"name": 1}},
    {"$skip": 20},
    {"$limit": 10}
])
```

---

## Accumulator Operators

Used within `$group` stage to compute aggregated values.

### $sum

**Count documents:**
```python
{"$sum": 1}  # Count each document as 1
```

**Sum field values:**
```python
{"$sum": "$salary"}  # Sum of all salaries
```

### $avg

**Average of field values:**
```python
{"$avg": "$age"}  # Average age
{"$avg": "$salary"}  # Average salary
```

### $min

**Minimum value:**
```python
{"$min": "$age"}  # Youngest age
{"$min": "$salary"}  # Lowest salary
```

### $max

**Maximum value:**
```python
{"$max": "$age"}  # Oldest age
{"$max": "$salary"}  # Highest salary
```

### $first

**First value in group:**
```python
{"$first": "$name"}  # First name in group
```

Note: Order depends on document insertion or prior `$sort` stage.

### $last

**Last value in group:**
```python
{"$last": "$name"}  # Last name in group
```

---

## Real-World Examples

### Example 1: Sales Analytics

```python
# Sample data
sales = db.collection("sales")
sales.insert_many([
    {"product": "Laptop", "category": "Electronics", "quantity": 5, "price": 1000, "date": "2024-01"},
    {"product": "Mouse", "category": "Electronics", "quantity": 20, "price": 25, "date": "2024-01"},
    {"product": "Desk", "category": "Furniture", "quantity": 3, "price": 500, "date": "2024-01"},
    # ... more sales
])

# Total revenue by category
results = sales.aggregate([
    {"$group": {
        "_id": "$category",
        "totalRevenue": {"$sum": {"$multiply": ["$quantity", "$price"]}},  # Note: currently not supported, use separate calculation
        "itemsSold": {"$sum": "$quantity"},
        "avgPrice": {"$avg": "$price"}
    }},
    {"$sort": {"totalRevenue": -1}}
])
```

### Example 2: User Demographics

```python
# Age distribution
results = users.aggregate([
    {"$group": {
        "_id": None,
        "totalUsers": {"$sum": 1},
        "avgAge": {"$avg": "$age"},
        "youngestAge": {"$min": "$age"},
        "oldestAge": {"$max": "$age"}
    }}
])

# Output:
# [{"_id": null, "totalUsers": 1000, "avgAge": 32.5, "youngestAge": 18, "oldestAge": 75}]
```

### Example 3: Department Report

```python
employees = db.collection("employees")

results = employees.aggregate([
    {"$match": {"status": "active"}},  # Only active employees
    {"$group": {
        "_id": "$department",
        "employees": {"$sum": 1},
        "avgSalary": {"$avg": "$salary"},
        "totalPayroll": {"$sum": "$salary"}
    }},
    {"$project": {
        "department": "$_id",
        "employees": 1,
        "avgSalary": 1,
        "totalPayroll": 1,
        "_id": 0
    }},
    {"$sort": {"totalPayroll": -1}}
])

# Output:
# [
#   {"department": "Engineering", "employees": 50, "avgSalary": 95000, "totalPayroll": 4750000},
#   {"department": "Sales", "employees": 30, "avgSalary": 75000, "totalPayroll": 2250000},
#   ...
# ]
```

### Example 4: Top N Analysis

```python
# Top 5 highest-paid employees per department
results = employees.aggregate([
    {"$sort": {"salary": -1}},
    {"$group": {
        "_id": "$department",
        "topEarners": {"$first": "$name"},  # First (highest) after sort
        "topSalary": {"$first": "$salary"}
    }},
    {"$limit": 5}
])
```

### Example 5: Complex Multi-Stage Pipeline

```python
# Comprehensive user analysis
results = users.aggregate([
    # Stage 1: Filter active users aged 25-50
    {"$match": {
        "status": "active",
        "age": {"$gte": 25, "$lte": 50}
    }},

    # Stage 2: Group by city and calculate stats
    {"$group": {
        "_id": "$city",
        "userCount": {"$sum": 1},
        "avgAge": {"$avg": "$age"},
        "avgIncome": {"$avg": "$income"}
    }},

    # Stage 3: Filter cities with 10+ users
    {"$match": {
        "userCount": {"$gte": 10}
    }},

    # Stage 4: Sort by average income
    {"$sort": {"avgIncome": -1}},

    # Stage 5: Get top 10 cities
    {"$limit": 10},

    # Stage 6: Reshape output
    {"$project": {
        "city": "$_id",
        "users": "$userCount",
        "avgAge": 1,
        "avgIncome": 1,
        "_id": 0
    }}
])
```

---

## Performance Tips

1. **Place $match early**: Filter documents as early as possible to reduce processing
   ```python
   # Good
   [{"$match": ...}, {"$group": ...}]

   # Bad (processes all docs before filtering)
   [{"$group": ...}, {"$match": ...}]
   ```

2. **Use indexes**: Ensure fields used in `$match` are indexed
   ```python
   collection.create_index("age")
   collection.aggregate([{"$match": {"age": {"$gte": 30}}}])
   ```

3. **Limit early**: Use `$limit` before expensive operations when possible

4. **Project only needed fields**: Reduce memory usage by projecting early
   ```python
   [
       {"$match": ...},
       {"$project": {"name": 1, "age": 1}},  # Only keep needed fields
       {"$group": ...}
   ]
   ```

---

## Current Limitations

- **Expression operators**: Currently only field references (`"$field"`) are supported
  - Not yet supported: `$multiply`, `$add`, `$subtract`, etc.
- **Nested field access**: `"$address.city"` not yet supported
- **Array operators**: `$unwind`, `$push`, etc. not yet implemented
- **Additional stages**: `$lookup`, `$facet`, `$bucket` not yet available

These features are planned for future releases.

---

## MongoDB Compatibility

MongoLite's aggregation pipeline is designed to be compatible with MongoDB's aggregation framework. Most basic pipelines should work identically.

**Differences:**
- MongoLite currently supports a subset of operators and stages
- Performance characteristics differ (MongoLite is optimized for embedded use)
- Some advanced features are not yet implemented

---

## Next Steps

- See [test_aggregation.py](test_aggregation.py) for comprehensive examples
- Check [IMPLEMENTATION_AGGREGATION.md](IMPLEMENTATION_AGGREGATION.md) for technical details
- Explore the [MongoDB Aggregation documentation](https://docs.mongodb.com/manual/aggregation/) for more complex patterns

---

**Questions or feature requests?** Open an issue on GitHub!
