# Aggregation Pipeline

IronBase supports MongoDB-compatible aggregation pipelines for data transformation and analysis.

## Table of Contents

- [Overview](#overview)
- [Pipeline Stages](#pipeline-stages)
- [Accumulators](#accumulators)
- [Dot Notation](#dot-notation)
- [Examples](#examples)
- [Performance Tips](#performance-tips)
- [Limitations](#limitations)

## Overview

An aggregation pipeline processes documents through a sequence of stages. Each stage transforms the data and passes results to the next stage.

```python
results = collection.aggregate([
    {"$match": {...}},      # Stage 1: Filter
    {"$group": {...}},      # Stage 2: Group & aggregate
    {"$project": {...}},    # Stage 3: Reshape
    {"$sort": {...}},       # Stage 4: Sort
    {"$limit": 10}          # Stage 5: Limit
])
```

## Pipeline Stages

### $match - Filter Documents

Filters documents like `find()`. Use early in the pipeline to reduce processing.

```python
# Filter by field value
{"$match": {"status": "active"}}

# Filter with operators
{"$match": {"age": {"$gte": 18, "$lt": 65}}}

# Multiple conditions (implicit AND)
{"$match": {"city": "NYC", "status": "active"}}

# Logical operators
{"$match": {"$or": [{"city": "NYC"}, {"city": "LA"}]}}
```

**Supported operators:** All query operators (`$eq`, `$gt`, `$in`, `$and`, `$or`, `$regex`, etc.)

### $group - Group & Aggregate

Groups documents by a key and computes aggregate values.

```python
{"$group": {
    "_id": <expression>,           # Group key
    "<field>": {<accumulator>}     # Computed fields
}}
```

**Group key options:**

```python
# Group by single field
{"$group": {"_id": "$city", ...}}

# Group all documents (no grouping)
{"$group": {"_id": null, ...}}

# Group by nested field (dot notation)
{"$group": {"_id": "$address.city", ...}}
```

**Example:**

```python
# Count users per city with stats
{"$group": {
    "_id": "$city",
    "count": {"$sum": 1},
    "avgAge": {"$avg": "$age"},
    "maxAge": {"$max": "$age"},
    "youngest": {"$first": "$name"}
}}
```

### $project - Reshape Documents

Include, exclude, or rename fields.

```python
# Include specific fields
{"$project": {"name": 1, "age": 1, "_id": 0}}

# Exclude fields
{"$project": {"password": 0, "ssn": 0}}

# Rename fields (copy value)
{"$project": {
    "userName": "$name",           # Rename name -> userName
    "location": "$address.city",   # Nested field access
    "score": 1                     # Keep original
}}
```

**Rules:**
- Include mode: `{"field": 1}` - only specified fields returned
- Exclude mode: `{"field": 0}` - all except specified returned
- Cannot mix include/exclude (except `_id: 0` in include mode)

### $sort - Sort Documents

```python
# Ascending
{"$sort": {"age": 1}}

# Descending
{"$sort": {"score": -1}}

# Multi-field (priority order)
{"$sort": {"city": 1, "age": -1}}

# Nested field
{"$sort": {"address.zip": 1}}
```

### $limit - Limit Results

```python
{"$limit": 10}
```

### $skip - Skip Documents

```python
{"$skip": 20}

# Pagination pattern: page 3 (20 skip, 10 per page)
[{"$skip": 20}, {"$limit": 10}]
```

## Accumulators

Used within `$group` to compute values across documents.

| Accumulator | Description | Example |
|-------------|-------------|---------|
| `$sum` | Sum values / count | `{"$sum": "$price"}` or `{"$sum": 1}` |
| `$avg` | Average | `{"$avg": "$score"}` |
| `$min` | Minimum | `{"$min": "$age"}` |
| `$max` | Maximum | `{"$max": "$salary"}` |
| `$first` | First in group | `{"$first": "$name"}` |
| `$last` | Last in group | `{"$last": "$timestamp"}` |

**All accumulators support dot notation:**

```python
{"$group": {
    "_id": "$location.country",
    "totalRevenue": {"$sum": "$payment.amount"},
    "avgRating": {"$avg": "$stats.rating"},
    "topCity": {"$first": "$address.city"}
}}
```

## Dot Notation

All aggregation features support MongoDB-style dot notation for nested fields.

### $group with nested _id

```python
# Group by nested field
{"$group": {
    "_id": "$address.city",
    "count": {"$sum": 1}
}}

# Deeply nested
{"$group": {
    "_id": "$store.location.region",
    "sales": {"$sum": "$payment.amount"}
}}
```

### Accumulators with nested fields

```python
{"$group": {
    "_id": "$category",
    "totalScore": {"$sum": "$stats.score"},
    "avgRating": {"$avg": "$reviews.rating"},
    "minPrice": {"$min": "$pricing.base"},
    "maxPrice": {"$max": "$pricing.premium"},
    "firstCity": {"$first": "$store.city"},
    "lastUpdate": {"$last": "$metadata.timestamp"}
}}
```

### $project with nested fields

```python
{"$project": {
    "city": "$address.city",
    "score": "$stats.totalScore",
    "_id": 0
}}
```

### $sort with nested fields

```python
{"$sort": {"address.zip": 1}}
{"$sort": {"stats.score": -1, "profile.age": 1}}
```

## Examples

### Sales Analytics

```python
sales = db.collection("sales")

# Revenue by product category
results = sales.aggregate([
    {"$match": {"status": "completed", "year": 2024}},
    {"$group": {
        "_id": "$product.category",
        "revenue": {"$sum": "$amount"},
        "orders": {"$sum": 1},
        "avgOrder": {"$avg": "$amount"}
    }},
    {"$sort": {"revenue": -1}},
    {"$limit": 10}
])
```

### User Demographics

```python
users = db.collection("users")

# Age distribution by city
results = users.aggregate([
    {"$match": {"status": "active"}},
    {"$group": {
        "_id": "$address.city",
        "userCount": {"$sum": 1},
        "avgAge": {"$avg": "$age"},
        "youngest": {"$min": "$age"},
        "oldest": {"$max": "$age"}
    }},
    {"$match": {"userCount": {"$gte": 100}}},  # Cities with 100+ users
    {"$sort": {"userCount": -1}},
    {"$project": {
        "city": "$_id",
        "users": "$userCount",
        "avgAge": 1,
        "ageRange": {"min": "$youngest", "max": "$oldest"},
        "_id": 0
    }}
])
```

### Time Series Analysis

```python
events = db.collection("events")

# Events per day
results = events.aggregate([
    {"$match": {"type": "purchase"}},
    {"$group": {
        "_id": "$date",
        "eventCount": {"$sum": 1},
        "totalValue": {"$sum": "$value"},
        "avgValue": {"$avg": "$value"}
    }},
    {"$sort": {"_id": 1}}
])
```

### Multi-Stage Pipeline

```python
# Complete pipeline with all stages
results = orders.aggregate([
    # 1. Filter recent completed orders
    {"$match": {
        "status": "completed",
        "date": {"$gte": "2024-01-01"}
    }},

    # 2. Group by store location
    {"$group": {
        "_id": "$store.location.city",
        "totalRevenue": {"$sum": "$payment.total"},
        "orderCount": {"$sum": 1},
        "avgOrder": {"$avg": "$payment.total"},
        "maxOrder": {"$max": "$payment.total"}
    }},

    # 3. Filter high-volume stores
    {"$match": {"orderCount": {"$gte": 50}}},

    # 4. Reshape output
    {"$project": {
        "city": "$_id",
        "revenue": "$totalRevenue",
        "orders": "$orderCount",
        "avgOrder": 1,
        "maxOrder": 1,
        "_id": 0
    }},

    # 5. Sort by revenue
    {"$sort": {"revenue": -1}},

    # 6. Top 20
    {"$limit": 20}
])
```

## Performance Tips

### 1. $match Early

Filter documents early to reduce processing:

```python
# Good - filters first
[{"$match": {"status": "active"}}, {"$group": ...}]

# Bad - processes all documents
[{"$group": ...}, {"$match": {"count": {"$gt": 10}}}]
```

### 2. Use Indexes

Create indexes on fields used in `$match`:

```python
collection.create_index("status")
collection.create_index("date")

# This $match uses indexes
{"$match": {"status": "active", "date": {"$gte": "2024-01-01"}}}
```

### 3. Project Early

Remove unneeded fields to reduce memory:

```python
[
    {"$match": {...}},
    {"$project": {"name": 1, "amount": 1}},  # Keep only needed
    {"$group": {...}}
]
```

### 4. Limit Early When Possible

If you need top N after sort:

```python
# Good
[{"$sort": {"score": -1}}, {"$limit": 10}]

# Also good - combined operation
[{"$sort": {"score": -1}}, {"$limit": 10}, {"$project": {...}}]
```

## Limitations

### Not Yet Supported

| Feature | Status |
|---------|--------|
| Expression operators (`$add`, `$multiply`) | Planned |
| `$unwind` (array expansion) | Planned |
| `$lookup` (joins) | Planned |
| `$facet` (parallel pipelines) | Planned |
| `$bucket` / `$bucketAuto` | Planned |
| Date operators | Planned |

### Current Behavior

- Field references only (`"$field"`) - no computed expressions
- Nested dot notation works everywhere
- All 6 stages and 6 accumulators fully implemented
- Pipeline processes all documents in memory

## MongoDB Compatibility

IronBase aggregation is designed for MongoDB compatibility. Most pipelines work identically:

```python
# Same syntax works in both MongoDB and IronBase
collection.aggregate([
    {"$match": {"status": "active"}},
    {"$group": {"_id": "$city", "count": {"$sum": 1}}},
    {"$sort": {"count": -1}},
    {"$limit": 10}
])
```

**Differences:**
- Subset of operators (see limitations)
- In-memory processing (no server-side cursors)
- Optimized for embedded use cases
