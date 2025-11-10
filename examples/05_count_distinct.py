#!/usr/bin/env python3
"""
Example 5: Count and Distinct Operations

Demonstrates aggregation-like operations in MongoLite:
- count_documents() - Count documents matching a filter
- distinct() - Get unique values from a field

Run: python examples/05_count_distinct.py
"""
import ironbase
import os

DB_FILE = "examples/data/count_distinct_example.db"

def setup():
    """Create examples/data directory if it doesn't exist"""
    os.makedirs("examples/data", exist_ok=True)
    if os.path.exists(DB_FILE):
        os.remove(DB_FILE)

def main():
    setup()

    print("="*60)
    print("MongoLite Example 5: Count and Distinct Operations")
    print("="*60 + "\n")

    db = ironbase.MongoLite(DB_FILE)
    orders = db.collection("orders")

    # Insert test data - E-commerce orders
    print("Inserting order data...\n")
    result = orders.insert_many([
        {"order_id": 1001, "customer": "Alice", "product": "Laptop", "category": "Electronics", "amount": 1200, "status": "shipped"},
        {"order_id": 1002, "customer": "Bob", "product": "Mouse", "category": "Electronics", "amount": 25, "status": "delivered"},
        {"order_id": 1003, "customer": "Alice", "product": "Desk", "category": "Furniture", "amount": 300, "status": "shipped"},
        {"order_id": 1004, "customer": "Carol", "product": "Chair", "category": "Furniture", "amount": 150, "status": "delivered"},
        {"order_id": 1005, "customer": "Bob", "product": "Monitor", "category": "Electronics", "amount": 400, "status": "pending"},
        {"order_id": 1006, "customer": "Alice", "product": "Keyboard", "category": "Electronics", "amount": 80, "status": "shipped"},
        {"order_id": 1007, "customer": "Dave", "product": "Lamp", "category": "Furniture", "amount": 60, "status": "delivered"},
        {"order_id": 1008, "customer": "Carol", "product": "Notebook", "category": "Stationery", "amount": 5, "status": "pending"},
        {"order_id": 1009, "customer": "Eve", "product": "Tablet", "category": "Electronics", "amount": 500, "status": "delivered"},
        {"order_id": 1010, "customer": "Alice", "product": "Pen Set", "category": "Stationery", "amount": 15, "status": "shipped"},
    ])
    print(f"insert_many result: {result}\n")

    # ==================== count_documents() - Basic ====================
    print("1. count_documents() - Count all documents\n")

    total = orders.count_documents({})
    print(f"Total orders in database: {total}\n")

    # ==================== count_documents() - With Filter ====================
    print("2. count_documents() - Count with simple filter\n")

    electronics_count = orders.count_documents({"category": "Electronics"})
    furniture_count = orders.count_documents({"category": "Furniture"})
    stationery_count = orders.count_documents({"category": "Stationery"})

    print("Orders by category:")
    print(f"  • Electronics: {electronics_count}")
    print(f"  • Furniture: {furniture_count}")
    print(f"  • Stationery: {stationery_count}")
    print()

    # ==================== count_documents() - Complex Filter ====================
    print("3. count_documents() - Count with complex filters\n")

    # Count high-value orders (> $100)
    high_value = orders.count_documents({"amount": {"$gt": 100}})
    print(f"High-value orders (> $100): {high_value}")

    # Count pending orders
    pending = orders.count_documents({"status": "pending"})
    print(f"Pending orders: {pending}")

    # Count delivered Electronics
    delivered_electronics = orders.count_documents({
        "$and": [
            {"category": "Electronics"},
            {"status": "delivered"}
        ]
    })
    print(f"Delivered Electronics orders: {delivered_electronics}")
    print()

    # ==================== count_documents() - By Customer ====================
    print("4. Counting orders per customer\n")

    # Get unique customers first
    customers = orders.distinct("customer")

    print("Orders per customer:")
    for customer in sorted(customers):
        count = orders.count_documents({"customer": customer})
        print(f"  • {customer}: {count} order(s)")
    print()

    # ==================== distinct() - Basic ====================
    print("5. distinct() - Get unique values\n")

    # Get all unique categories
    categories = orders.distinct("category")
    print(f"Unique categories ({len(categories)}):")
    for cat in sorted(categories):
        print(f"  • {cat}")
    print()

    # Get all unique statuses
    statuses = orders.distinct("status")
    print(f"Unique order statuses ({len(statuses)}):")
    for status in sorted(statuses):
        print(f"  • {status}")
    print()

    # ==================== distinct() - With Filter ====================
    print("6. distinct() - Get unique values with filter\n")

    # Get unique customers who ordered Electronics
    electronics_customers = orders.distinct("customer", {"category": "Electronics"})
    print(f"Customers who bought Electronics ({len(electronics_customers)}):")
    for customer in sorted(electronics_customers):
        print(f"  • {customer}")
    print()

    # Get unique categories for high-value orders
    high_value_categories = orders.distinct("category", {"amount": {"$gt": 100}})
    print(f"Categories with high-value orders (> $100):")
    for cat in sorted(high_value_categories):
        print(f"  • {cat}")
    print()

    # ==================== Combining count and distinct ====================
    print("7. Analytics - Combining count and distinct\n")

    print("Sales Analytics Report")
    print("━" * 40)

    # Total metrics
    total_orders = orders.count_documents({})
    unique_customers = len(orders.distinct("customer"))
    unique_products = len(orders.distinct("product"))

    print(f"\nOverall Metrics:")
    print(f"  Total Orders: {total_orders}")
    print(f"  Unique Customers: {unique_customers}")
    print(f"  Unique Products: {unique_products}")

    # Status breakdown
    print(f"\nOrder Status Breakdown:")
    for status in ["pending", "shipped", "delivered"]:
        count = orders.count_documents({"status": status})
        percentage = (count / total_orders) * 100
        print(f"  • {status.capitalize()}: {count} ({percentage:.1f}%)")

    # Category analysis
    print(f"\nCategory Analysis:")
    categories = orders.distinct("category")
    for category in sorted(categories):
        count = orders.count_documents({"category": category})
        customers = len(orders.distinct("customer", {"category": category}))
        print(f"  • {category}:")
        print(f"    Orders: {count}")
        print(f"    Unique customers: {customers}")

    print()

    # ==================== Practical Example: Customer Insights ====================
    print("8. Practical example - Customer segmentation\n")

    print("Customer Segmentation Report")
    print("━" * 40)

    customers = orders.distinct("customer")

    vip_customers = []
    regular_customers = []
    new_customers = []

    for customer in customers:
        # Count orders per customer
        order_count = orders.count_documents({"customer": customer})

        # Get categories this customer ordered from
        customer_categories = orders.distinct("category", {"customer": customer})

        # Calculate total spent (we'll count orders as proxy)
        customer_orders = orders.find({"customer": customer})
        total_spent = sum(order["amount"] for order in customer_orders)

        # Segment customers
        if order_count >= 4:
            vip_customers.append((customer, order_count, total_spent))
        elif order_count >= 2:
            regular_customers.append((customer, order_count, total_spent))
        else:
            new_customers.append((customer, order_count, total_spent))

    print(f"\n🌟 VIP Customers ({len(vip_customers)}) - 4+ orders:")
    for customer, order_count, total_spent in sorted(vip_customers, key=lambda x: x[2], reverse=True):
        categories = orders.distinct("category", {"customer": customer})
        print(f"  • {customer}")
        print(f"    Orders: {order_count}")
        print(f"    Total spent: ${total_spent:,}")
        print(f"    Categories: {', '.join(sorted(categories))}")

    print(f"\n👤 Regular Customers ({len(regular_customers)}) - 2-3 orders:")
    for customer, order_count, total_spent in sorted(regular_customers):
        print(f"  • {customer}: {order_count} orders, ${total_spent:,} spent")

    print(f"\n🆕 New Customers ({len(new_customers)}) - 1 order:")
    for customer, order_count, total_spent in sorted(new_customers):
        print(f"  • {customer}: ${total_spent:,} spent")

    print()

    # ==================== Advanced: Category Cross-sell Analysis ====================
    print("9. Advanced - Category cross-sell analysis\n")

    print("Which customers buy from multiple categories?")
    print("━" * 40)

    customers = orders.distinct("customer")

    multi_category_buyers = []

    for customer in customers:
        categories = orders.distinct("category", {"customer": customer})
        if len(categories) > 1:
            order_count = orders.count_documents({"customer": customer})
            multi_category_buyers.append((customer, categories, order_count))

    if multi_category_buyers:
        print(f"\nFound {len(multi_category_buyers)} multi-category buyers:\n")
        for customer, categories, order_count in sorted(multi_category_buyers, key=lambda x: len(x[1]), reverse=True):
            print(f"  • {customer} ({order_count} orders)")
            print(f"    Buys from: {', '.join(sorted(categories))}")
    else:
        print("No multi-category buyers found")

    print()

    db.close()

    print("="*60)
    print("✅ Example completed successfully!")
    print("="*60)

if __name__ == "__main__":
    main()
