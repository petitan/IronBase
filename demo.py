#!/usr/bin/env python3
"""
MongoLite Demo - Showcase all implemented features

Run: python demo.py
"""
import ironbase
import os

DB_PATH = "demo.db"

def cleanup():
    """Remove demo database"""
    if os.path.exists(DB_PATH):
        os.remove(DB_PATH)

def print_section(title):
    """Print section header"""
    print(f"\n{'='*60}")
    print(f"  {title}")
    print(f"{'='*60}\n")

def demo_insert(db):
    """Demonstrate insert operations"""
    print_section("INSERT Operations")

    users = db.collection("users")

    # Insert one
    result = users.insert_one({
        "name": "Alice",
        "age": 25,
        "city": "NYC",
        "hobbies": ["reading", "coding"]
    })
    print(f"✓ insert_one: {result}")

    # Insert many
    result = users.insert_many([
        {"name": "Bob", "age": 30, "city": "LA", "active": True},
        {"name": "Charlie", "age": 25, "city": "NYC", "active": False},
        {"name": "Dave", "age": 35, "city": "Chicago", "active": True},
        {"name": "Eve", "age": 28, "city": "LA", "active": True},
    ])
    print(f"✓ insert_many: {result}")

def demo_find(db):
    """Demonstrate find operations"""
    print_section("FIND Operations")

    users = db.collection("users")

    # Find all
    print("All users:")
    for user in users.find({}):
        print(f"  - {user['name']}, {user['age']}, {user['city']}")

    # Find with filter
    print("\nUsers age >= 30:")
    for user in users.find({"age": {"$gte": 30}}):
        print(f"  - {user['name']}, {user['age']}")

    # Find with complex query
    print("\nUsers in NYC OR age > 30:")
    results = users.find({
        "$or": [
            {"city": "NYC"},
            {"age": {"$gt": 30}}
        ]
    })
    for user in results:
        print(f"  - {user['name']}, {user['age']}, {user['city']}")

    # Find one
    print("\nFind one user named Bob:")
    user = users.find_one({"name": "Bob"})
    print(f"  {user}")


def demo_count(db):
    """Demonstrate count operation"""
    print_section("COUNT Operations")

    users = db.collection("users")

    # Count all
    total = users.count_documents({})
    print(f"Total users: {total}")

    # Count with filter
    nyc_users = users.count_documents({"city": "NYC"})
    print(f"Users in NYC: {nyc_users}")

    # Count with complex filter
    active_young = users.count_documents({
        "$and": [
            {"active": True},
            {"age": {"$lt": 30}}
        ]
    })
    print(f"Active users under 30: {active_young}")


def demo_distinct(db):
    """Demonstrate distinct operation"""
    print_section("DISTINCT Operations")

    users = db.collection("users")

    # Get distinct ages
    ages = users.distinct("age")
    print(f"Unique ages: {sorted(ages)}")

    # Get distinct cities
    cities = users.distinct("city")
    print(f"Unique cities: {sorted(cities)}")

    # Get distinct ages for active users
    active_ages = users.distinct("age", {"active": True})
    print(f"Unique ages (active users only): {sorted(active_ages)}")


def demo_update(db):
    """Demonstrate update operations"""
    print_section("UPDATE Operations")

    users = db.collection("users")

    # Update one - $set
    result = users.update_one(
        {"name": "Alice"},
        {"$set": {"city": "Boston", "updated": True}}
    )
    print(f"✓ update_one ($set): {result}")

    # Update one - $inc
    result = users.update_one(
        {"name": "Bob"},
        {"$inc": {"age": 1}}
    )
    print(f"✓ update_one ($inc): {result}")

    # Update many
    result = users.update_many(
        {"city": "LA"},
        {"$set": {"verified": True}}
    )
    print(f"✓ update_many: {result}")

    # Show updated data
    print("\nAlice after update:")
    alice = users.find_one({"name": "Alice"})
    print(f"  {alice}")


def demo_delete(db):
    """Demonstrate delete operations"""
    print_section("DELETE Operations")

    users = db.collection("users")

    print(f"Users before delete: {users.count_documents({})}")

    # Delete one
    result = users.delete_one({"name": "Charlie"})
    print(f"✓ delete_one: {result}")

    print(f"Users after delete_one: {users.count_documents({})}")

    # Delete many
    result = users.delete_many({"active": False})
    print(f"✓ delete_many: {result}")

    print(f"Users after delete_many: {users.count_documents({})}")

    # Show remaining users
    print("\nRemaining users:")
    for user in users.find({}):
        print(f"  - {user['name']}")


def demo_complex_queries(db):
    """Demonstrate complex query combinations"""
    print_section("COMPLEX QUERIES")

    # Recreate data for complex queries
    products = db.collection("products")
    products.insert_many([
        {"name": "Laptop", "category": "Electronics", "price": 1200, "inStock": True},
        {"name": "Mouse", "category": "Electronics", "price": 25, "inStock": True},
        {"name": "Desk", "category": "Furniture", "price": 300, "inStock": False},
        {"name": "Chair", "category": "Furniture", "price": 150, "inStock": True},
        {"name": "Monitor", "category": "Electronics", "price": 400, "inStock": True},
        {"name": "Keyboard", "category": "Electronics", "price": 80, "inStock": False},
    ])

    # Complex query: Electronics OR Furniture, AND in stock, AND price < 500
    print("Query: (Electronics OR Furniture) AND inStock AND price < 500")
    results = products.find({
        "$and": [
            {
                "$or": [
                    {"category": "Electronics"},
                    {"category": "Furniture"}
                ]
            },
            {"inStock": True},
            {"price": {"$lt": 500}}
        ]
    })

    for product in results:
        print(f"  - {product['name']}: ${product['price']} ({product['category']})")

    # NOT query
    print("\nQuery: NOT price > 300")
    results = products.find({
        "price": {"$not": {"$gt": 300}}
    })

    for product in results:
        print(f"  - {product['name']}: ${product['price']}")


def main():
    """Run all demos"""
    print("\n" + "="*60)
    print("  MongoLite Feature Demo")
    print("  All operations with test-first development")
    print("="*60)

    cleanup()

    try:
        db = ironbase.MongoLite(DB_PATH)

        demo_insert(db)
        demo_find(db)
        demo_count(db)
        demo_distinct(db)
        demo_update(db)
        demo_delete(db)
        demo_complex_queries(db)

        db.close()

        print("\n" + "="*60)
        print("  ✅ All demos completed successfully!")
        print("="*60 + "\n")

    finally:
        cleanup()

if __name__ == "__main__":
    main()
