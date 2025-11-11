#!/usr/bin/env python3
"""
Example 2: Query Operators

Demonstrates all comparison operators in IronBase:
- $eq (equal), $ne (not equal)
- $gt (greater than), $gte (greater than or equal)
- $lt (less than), $lte (less than or equal)
- $in (in array), $nin (not in array)

Run: python examples/02_query_operators.py
"""
import ironbase
import os

DB_FILE = "examples/data/query_ops_example.db"

def setup():
    """Create examples/data directory if it doesn't exist"""
    os.makedirs("examples/data", exist_ok=True)
    if os.path.exists(DB_FILE):
        os.remove(DB_FILE)

def main():
    setup()

    print("="*60)
    print("IronBase Example 2: Query Operators")
    print("="*60 + "\n")

    db = ironbase.IronBase(DB_FILE)
    products = db.collection("products")

    # Insert test data
    print("Inserting product data...\n")
    products.insert_many([
        {"name": "Laptop", "category": "Electronics", "price": 1200, "stock": 15},
        {"name": "Mouse", "category": "Electronics", "price": 25, "stock": 100},
        {"name": "Keyboard", "category": "Electronics", "price": 80, "stock": 50},
        {"name": "Monitor", "category": "Electronics", "price": 400, "stock": 30},
        {"name": "Desk", "category": "Furniture", "price": 300, "stock": 20},
        {"name": "Chair", "category": "Furniture", "price": 150, "stock": 45},
        {"name": "Lamp", "category": "Furniture", "price": 60, "stock": 80},
        {"name": "Notebook", "category": "Stationery", "price": 5, "stock": 200},
    ])

    # ==================== $eq (Equal) ====================
    print("1. $eq (Equal) - Find products with price exactly 80\n")
    results = products.find({"price": {"$eq": 80}})
    # Note: {"price": 80} is equivalent to {"price": {"$eq": 80}}
    print(f"Found {len(results)} product(s):")
    for product in results:
        print(f"  • {product['name']} - ${product['price']}")
    print()

    # ==================== $ne (Not Equal) ====================
    print("2. $ne (Not Equal) - Find products NOT in Electronics category\n")
    results = products.find({"category": {"$ne": "Electronics"}})
    print(f"Found {len(results)} product(s):")
    for product in results:
        print(f"  • {product['name']} ({product['category']})")
    print()

    # ==================== $gt (Greater Than) ====================
    print("3. $gt (Greater Than) - Find products with price > 100\n")
    results = products.find({"price": {"$gt": 100}})
    print(f"Found {len(results)} product(s):")
    for product in results:
        print(f"  • {product['name']} - ${product['price']}")
    print()

    # ==================== $gte (Greater Than or Equal) ====================
    print("4. $gte (Greater Than or Equal) - Find products with price >= 100\n")
    results = products.find({"price": {"$gte": 100}})
    print(f"Found {len(results)} product(s):")
    for product in results:
        print(f"  • {product['name']} - ${product['price']}")
    print()

    # ==================== $lt (Less Than) ====================
    print("5. $lt (Less Than) - Find products with stock < 30\n")
    results = products.find({"stock": {"$lt": 30}})
    print(f"Found {len(results)} product(s):")
    for product in results:
        print(f"  • {product['name']} - Stock: {product['stock']}")
    print()

    # ==================== $lte (Less Than or Equal) ====================
    print("6. $lte (Less Than or Equal) - Find products with stock <= 30\n")
    results = products.find({"stock": {"$lte": 30}})
    print(f"Found {len(results)} product(s):")
    for product in results:
        print(f"  • {product['name']} - Stock: {product['stock']}")
    print()

    # ==================== $in (In Array) ====================
    print("7. $in (In Array) - Find products in specific categories\n")
    results = products.find({"category": {"$in": ["Furniture", "Stationery"]}})
    print(f"Found {len(results)} product(s) in Furniture or Stationery:")
    for product in results:
        print(f"  • {product['name']} ({product['category']})")
    print()

    # ==================== $nin (Not In Array) ====================
    print("8. $nin (Not In Array) - Find products NOT in specific price ranges\n")
    results = products.find({"price": {"$nin": [25, 80, 150]}})
    print(f"Found {len(results)} product(s) with non-standard pricing:")
    for product in results:
        print(f"  • {product['name']} - ${product['price']}")
    print()

    # ==================== Combining Operators ====================
    print("9. Combining Operators - Electronics with price between 50 and 500\n")
    results = products.find({
        "category": "Electronics",
        "price": {"$gte": 50, "$lte": 500}
    })
    print(f"Found {len(results)} product(s):")
    for product in results:
        print(f"  • {product['name']} - ${product['price']}")
    print()

    # ==================== Practical Example ====================
    print("10. Practical Query - Low stock items that need reordering\n")
    print("Find Electronics with stock < 50 and price > 50 (valuable items)")
    results = products.find({
        "category": "Electronics",
        "stock": {"$lt": 50},
        "price": {"$gt": 50}
    })
    print(f"\n⚠️  {len(results)} item(s) need attention:")
    for product in results:
        print(f"  • {product['name']}")
        print(f"    Price: ${product['price']}")
        print(f"    Current stock: {product['stock']} units")
        print(f"    Action: Reorder soon!")
    print()

    db.close()

    print("="*60)
    print("✅ Example completed successfully!")
    print("="*60)

if __name__ == "__main__":
    main()
