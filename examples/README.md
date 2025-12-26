# IronBase Examples

Comprehensive example programs demonstrating all IronBase features.

## Overview

This directory contains 6 example programs that showcase IronBase's capabilities:

| Example | Topic | Description |
|---------|-------|-------------|
| `01_basic_crud.py` | **CRUD Operations** | Create, Read, Update, Delete basics |
| `02_query_operators.py` | **Query Operators** | All comparison operators ($gt, $lt, $in, etc.) |
| `03_logical_operators.py` | **Logical Operators** | Complex queries with $and, $or, $not, $nor |
| `04_update_operators.py` | **Update Operators** | $set, $inc, $unset with practical examples |
| `05_count_distinct.py` | **Aggregation** | count_documents() and distinct() operations |
| `06_real_world_blog.py` | **Real-World App** | Complete blog system with multiple collections |

## Running the Examples

All examples are standalone Python scripts that can be run directly:

```bash
# Make sure IronBase is installed
maturin develop

# Run any example
python examples/01_basic_crud.py
python examples/02_query_operators.py
# ... and so on
```

Each example will:
- Create its own database file in `examples/data/`
- Print detailed output showing operations and results
- Clean up after itself

## Example Details

### 1. Basic CRUD Operations (`01_basic_crud.py`)

Learn the fundamental operations for working with IronBase:

**Topics covered:**
- Opening a database and accessing collections
- `insert_one()` and `insert_many()` - Creating documents
- `find()` and `find_one()` - Reading/querying documents
- `update_one()` and `update_many()` - Modifying documents
- `delete_one()` and `delete_many()` - Removing documents
- `count_documents()` - Counting documents

**Perfect for:** Complete beginners wanting to understand the basics.

```python
# Quick example from 01_basic_crud.py
db = ironbase.IronBase("mydb.db")
users = db.collection("users")

# Insert
users.insert_one({"name": "Alice", "age": 28})

# Find
alice = users.find_one({"name": "Alice"})

# Update
users.update_one({"name": "Alice"}, {"$set": {"age": 29}})

# Delete
users.delete_one({"name": "Alice"})
```

### 2. Query Operators (`02_query_operators.py`)

Master all comparison operators for precise queries:

**Operators covered:**
- `$eq` - Equal to
- `$ne` - Not equal to
- `$gt` - Greater than
- `$gte` - Greater than or equal
- `$lt` - Less than
- `$lte` - Less than or equal
- `$in` - Value in array
- `$nin` - Value not in array

**Perfect for:** Learning to filter and search documents effectively.

```python
# Examples from 02_query_operators.py

# Find products with price > 100
expensive = products.find({"price": {"$gt": 100}})

# Find products in specific categories
furniture = products.find({"category": {"$in": ["Furniture", "Stationery"]}})

# Combine operators
mid_range = products.find({
    "price": {"$gte": 50, "$lte": 500}
})
```

### 3. Logical Operators (`03_logical_operators.py`)

Build complex queries with logical operators:

**Operators covered:**
- `$and` - All conditions must match
- `$or` - At least one condition must match
- `$not` - Negates a condition
- `$nor` - None of the conditions must match

**Perfect for:** Complex business logic and advanced filtering.

```python
# Examples from 03_logical_operators.py

# Find active engineers earning over $80k
results = employees.find({
    "$and": [
        {"department": "Engineering"},
        {"active": True},
        {"salary": {"$gt": 80000}}
    ]
})

# Find young OR high earners
results = employees.find({
    "$or": [
        {"age": {"$lt": 30}},
        {"salary": {"$gt": 90000}}
    ]
})

# Find NOT older than 35
results = employees.find({
    "age": {"$not": {"$gt": 35}}
})
```

### 4. Update Operators (`04_update_operators.py`)

Learn all the ways to modify documents:

**Operators covered:**
- `$set` - Set/update field values
- `$inc` - Increment/decrement numeric fields
- `$unset` - Remove fields from documents

**Perfect for:** Understanding document modifications and field management.

```python
# Examples from 04_update_operators.py

# Set fields (update existing, add new)
users.update_one(
    {"name": "Alice"},
    {"$set": {"age": 29, "city": "NYC"}}
)

# Increment score, decrement lives
users.update_one(
    {"name": "Bob"},
    {"$inc": {"score": 50, "lives": -1}}
)

# Remove field
users.update_one(
    {"name": "Carol"},
    {"$unset": {"temp_field": ""}}
)

# Combine multiple operators
users.update_one(
    {"name": "Dave"},
    {
        "$set": {"status": "active"},
        "$inc": {"score": 100},
        "$unset": {"old_field": ""}
    }
)
```

### 5. Count and Distinct (`05_count_distinct.py`)

Perform aggregation-like operations:

**Operations covered:**
- `count_documents()` - Count with filters
- `distinct()` - Get unique values from a field

**Perfect for:** Analytics, reporting, and understanding data distribution.

```python
# Examples from 05_count_distinct.py

# Count all documents
total = orders.count_documents({})

# Count with filter
electronics = orders.count_documents({"category": "Electronics"})

# Complex count
delivered = orders.count_documents({
    "$and": [
        {"category": "Electronics"},
        {"status": "delivered"}
    ]
})

# Get unique values
categories = orders.distinct("category")
# Result: ["Electronics", "Furniture", "Stationery"]

# Distinct with filter
electronics_customers = orders.distinct(
    "customer",
    {"category": "Electronics"}
)

# Combine for analytics
for category in categories:
    count = orders.count_documents({"category": category})
    customers = len(orders.distinct("customer", {"category": category}))
    print(f"{category}: {count} orders from {customers} customers")
```

### 6. Real-World Blog System (`06_real_world_blog.py`)

Complete application demonstrating real-world usage:

**Features demonstrated:**
- Multiple collections (users, posts, comments)
- Document relationships (author_id references)
- Full CRUD operations in context
- Analytics and reporting
- Content management
- User activity tracking

**Perfect for:** Seeing how everything comes together in a real application.

**System includes:**
- User management (roles, activation)
- Post creation and publishing
- Comment system
- Tag-based organization
- View/like tracking
- Search functionality
- Analytics dashboard

```python
# Snippet from 06_real_world_blog.py

# Create user
users.insert_one({
    "username": "alice_dev",
    "email": "alice@example.com",
    "role": "admin",
    "active": True
})

# Create post
posts.insert_one({
    "title": "Getting Started with IronBase",
    "author_id": alice_id,
    "content": "...",
    "tags": ["database", "mongodb", "python"],
    "status": "published",
    "views": 0,
    "likes": 0
})

# Add comment
comments.insert_one({
    "post_id": post_id,
    "author_id": dave_id,
    "content": "Great tutorial!",
    "likes": 0
})

# Track engagement
posts.update_one(
    {"_id": post_id},
    {"$inc": {"views": 1, "likes": 1}}
)

# Analytics
total_posts = posts.count_documents({"status": "published"})
top_tags = posts.distinct("tags", {"status": "published"})
```

## Quick Reference

### CRUD Operations

```python
# INSERT
collection.insert_one(document)
collection.insert_many([doc1, doc2, ...])

# READ
collection.find(query)
collection.find_one(query)
collection.count_documents(query)
collection.distinct(field, query)

# UPDATE
collection.update_one(query, update)
collection.update_many(query, update)

# DELETE
collection.delete_one(query)
collection.delete_many(query)
```

### Query Operators

```python
{"field": value}                    # Equal
{"field": {"$eq": value}}          # Equal (explicit)
{"field": {"$ne": value}}          # Not equal
{"field": {"$gt": value}}          # Greater than
{"field": {"$gte": value}}         # Greater than or equal
{"field": {"$lt": value}}          # Less than
{"field": {"$lte": value}}         # Less than or equal
{"field": {"$in": [v1, v2]}}      # In array
{"field": {"$nin": [v1, v2]}}     # Not in array
```

### Logical Operators

```python
{"$and": [query1, query2]}         # All must match
{"$or": [query1, query2]}          # At least one matches
{"field": {"$not": {operator}}}    # Negation
{"$nor": [query1, query2]}         # None must match
```

### Update Operators

```python
{"$set": {"field": value}}         # Set/update field
{"$inc": {"field": amount}}        # Increment/decrement
{"$unset": {"field": ""}}          # Remove field
```

## Database Files

All examples create database files in `examples/data/`:

- `crud_example.db` - From 01_basic_crud.py
- `query_ops_example.db` - From 02_query_operators.py
- `logical_ops_example.db` - From 03_logical_operators.py
- `update_ops_example.db` - From 04_update_operators.py
- `count_distinct_example.db` - From 05_count_distinct.py
- `blog_example.db` - From 06_real_world_blog.py

You can inspect these files or reuse them for experimentation.

## Learning Path

**Recommended order for learning:**

1. **Start with `01_basic_crud.py`** - Understand the fundamentals
2. **Move to `02_query_operators.py`** - Learn to filter data
3. **Then `03_logical_operators.py`** - Build complex queries
4. **Next `04_update_operators.py`** - Master data modification
5. **Then `05_count_distinct.py`** - Learn aggregation basics
6. **Finally `06_real_world_blog.py`** - See it all in action

## Additional Resources

- **Main README**: `../README.md` - Project overview and installation
- **Test Suite**: `../test_*.py` - Comprehensive tests for all features
- **Demo Script**: `../demo.py` - Quick feature demonstration

## API Documentation

For detailed API documentation, see the main README.md file.

## Need Help?

If you encounter issues or have questions:

1. Check the example code - it's heavily commented
2. Review the main README.md for API details
3. Look at the test files for more usage examples
4. Open an issue on GitHub

---

**IronBase** - MongoDB simplicity with embedded convenience
