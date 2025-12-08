#!/usr/bin/env python3
"""
Generate a realistic test database for IronBase TUI
- 100+ collections with various domains
- Some collections with 1000+ documents
- Realistic data patterns
"""

import sys
sys.path.insert(0, '/home/petitan/MongoLite')

import ironbase
import random
import string
from datetime import datetime, timedelta

# Configuration
DB_PATH = "/tmp/tui_large_test.mlite"
LARGE_COLLECTION_SIZE = 1500  # Documents in "large" collections
MEDIUM_COLLECTION_SIZE = 200
SMALL_COLLECTION_SIZE = 20

# Data generators
def random_string(length=10):
    return ''.join(random.choices(string.ascii_letters, k=length))

def random_email(name):
    domains = ["gmail.com", "yahoo.com", "outlook.com", "company.hu", "example.org"]
    return f"{name.lower()}@{random.choice(domains)}"

def random_date(start_year=2020):
    start = datetime(start_year, 1, 1)
    days = random.randint(0, 1500)
    return (start + timedelta(days=days)).isoformat()

def random_price():
    return round(random.uniform(10, 10000), 2)

# Collection templates
COLLECTION_TEMPLATES = {
    # Large collections (1500+ docs each)
    "users": lambda i: {
        "name": f"User_{i}_{random_string(5)}",
        "email": random_email(f"user{i}"),
        "age": random.randint(18, 80),
        "city": random.choice(["Budapest", "Debrecen", "Szeged", "Pecs", "Gyor", "Miskolc", "Kecskemet"]),
        "status": random.choice(["active", "inactive", "pending"]),
        "created_at": random_date(),
        "score": random.randint(0, 1000)
    },
    "products": lambda i: {
        "name": f"Product_{i}",
        "sku": f"SKU-{random_string(8).upper()}",
        "category": random.choice(["electronics", "furniture", "clothing", "food", "toys", "books", "sports"]),
        "price": random_price(),
        "stock": random.randint(0, 500),
        "active": random.choice([True, False]),
        "tags": random.sample(["sale", "new", "popular", "featured", "limited"], k=random.randint(0, 3))
    },
    "orders": lambda i: {
        "order_id": f"ORD-{i:06d}",
        "user_id": random.randint(1, 1000),
        "total": random_price(),
        "status": random.choice(["pending", "processing", "shipped", "delivered", "cancelled"]),
        "items": random.randint(1, 10),
        "created_at": random_date()
    },
    "logs": lambda i: {
        "timestamp": random_date(2023),
        "level": random.choice(["INFO", "WARN", "ERROR", "DEBUG"]),
        "service": random.choice(["api", "worker", "scheduler", "auth", "payment"]),
        "message": f"Log message {i}: {random_string(20)}",
        "request_id": random_string(16)
    },
    "events": lambda i: {
        "event_type": random.choice(["click", "view", "purchase", "signup", "logout"]),
        "user_id": random.randint(1, 1000),
        "timestamp": random_date(2023),
        "properties": {"page": f"/page/{random.randint(1, 100)}", "duration": random.randint(1, 300)}
    },

    # Medium collections (200 docs each)
    "categories": lambda i: {
        "name": f"Category_{i}",
        "slug": f"cat-{i}",
        "parent_id": random.randint(0, 50) if i > 10 else None,
        "level": random.randint(0, 3)
    },
    "tags": lambda i: {
        "name": f"Tag_{random_string(6)}",
        "color": f"#{random.randint(0, 0xFFFFFF):06x}",
        "count": random.randint(0, 1000)
    },
    "comments": lambda i: {
        "user_id": random.randint(1, 1000),
        "post_id": random.randint(1, 500),
        "text": f"Comment text {i}: {random_string(50)}",
        "likes": random.randint(0, 100),
        "created_at": random_date()
    },
    "reviews": lambda i: {
        "product_id": random.randint(1, 1000),
        "user_id": random.randint(1, 1000),
        "rating": random.randint(1, 5),
        "title": f"Review {i}",
        "text": random_string(100),
        "helpful_votes": random.randint(0, 50)
    },
    "notifications": lambda i: {
        "user_id": random.randint(1, 1000),
        "type": random.choice(["email", "push", "sms"]),
        "title": f"Notification {i}",
        "read": random.choice([True, False]),
        "created_at": random_date()
    },

    # Small collections (20 docs each) - various domains
    "settings": lambda i: {"key": f"setting_{i}", "value": random_string(20), "type": random.choice(["string", "number", "boolean"])},
    "roles": lambda i: {"name": f"Role_{i}", "permissions": random.sample(["read", "write", "delete", "admin"], k=random.randint(1, 4))},
    "countries": lambda i: {"code": random_string(2).upper(), "name": f"Country_{i}", "active": True},
    "currencies": lambda i: {"code": random_string(3).upper(), "symbol": random.choice(["$", "€", "£", "¥"]), "rate": random.uniform(0.5, 2.0)},
    "languages": lambda i: {"code": random_string(2).lower(), "name": f"Language_{i}", "rtl": random.choice([True, False])},
    "timezones": lambda i: {"name": f"Timezone_{i}", "offset": random.randint(-12, 12)},
    "payment_methods": lambda i: {"name": f"Payment_{i}", "type": random.choice(["card", "bank", "wallet"]), "active": True},
    "shipping_methods": lambda i: {"name": f"Shipping_{i}", "price": random_price(), "days": random.randint(1, 14)},
    "tax_rates": lambda i: {"country": random_string(2).upper(), "rate": random.uniform(0, 0.3), "name": f"Tax_{i}"},
    "warehouses": lambda i: {"name": f"Warehouse_{i}", "city": random.choice(["Budapest", "Wien", "Praha"]), "capacity": random.randint(1000, 50000)},
}

# Generate additional small collections for various business domains
DOMAIN_PREFIXES = [
    "customer", "vendor", "supplier", "partner", "employee", "department",
    "project", "task", "milestone", "sprint", "issue", "ticket",
    "invoice", "payment", "transaction", "refund", "credit",
    "campaign", "lead", "opportunity", "deal", "contact",
    "article", "post", "page", "media", "attachment",
    "report", "dashboard", "widget", "chart", "metric",
    "session", "token", "permission", "audit", "backup",
    "queue", "job", "worker", "schedule", "cron",
    "cache", "config", "feature", "flag", "experiment",
    "template", "email", "sms", "webhook", "integration",
    "api_key", "rate_limit", "quota", "usage", "billing",
    "subscription", "plan", "addon", "discount", "coupon",
    "inventory", "stock", "location", "transfer", "adjustment",
    "shipment", "tracking", "carrier", "route", "delivery"
]

def generate_generic_doc(prefix, i):
    """Generate a generic document for any collection"""
    return {
        "name": f"{prefix.title()}_{i}",
        "code": f"{prefix[:3].upper()}-{i:04d}",
        "status": random.choice(["active", "inactive", "pending", "archived"]),
        "priority": random.randint(1, 5),
        "created_at": random_date(),
        "updated_at": random_date(2023),
        "metadata": {
            "source": random.choice(["import", "api", "manual", "sync"]),
            "version": random.randint(1, 10)
        }
    }


def main():
    print(f"Creating database: {DB_PATH}")

    # Remove existing
    import os
    if os.path.exists(DB_PATH):
        os.remove(DB_PATH)

    db = ironbase.IronBase(DB_PATH)
    total_docs = 0

    # 1. Large collections (5 x 1500 = 7500 docs)
    large_collections = ["users", "products", "orders", "logs", "events"]
    for coll_name in large_collections:
        print(f"  Creating {coll_name} ({LARGE_COLLECTION_SIZE} docs)...")
        coll = db.collection(coll_name)
        template = COLLECTION_TEMPLATES[coll_name]
        docs = [template(i) for i in range(1, LARGE_COLLECTION_SIZE + 1)]
        coll.insert_many(docs)
        total_docs += LARGE_COLLECTION_SIZE

    # 2. Medium collections (5 x 200 = 1000 docs)
    medium_collections = ["categories", "tags", "comments", "reviews", "notifications"]
    for coll_name in medium_collections:
        print(f"  Creating {coll_name} ({MEDIUM_COLLECTION_SIZE} docs)...")
        coll = db.collection(coll_name)
        template = COLLECTION_TEMPLATES[coll_name]
        docs = [template(i) for i in range(1, MEDIUM_COLLECTION_SIZE + 1)]
        coll.insert_many(docs)
        total_docs += MEDIUM_COLLECTION_SIZE

    # 3. Small template collections (10 x 20 = 200 docs)
    small_template_collections = ["settings", "roles", "countries", "currencies", "languages",
                                   "timezones", "payment_methods", "shipping_methods", "tax_rates", "warehouses"]
    for coll_name in small_template_collections:
        print(f"  Creating {coll_name} ({SMALL_COLLECTION_SIZE} docs)...")
        coll = db.collection(coll_name)
        template = COLLECTION_TEMPLATES[coll_name]
        docs = [template(i) for i in range(1, SMALL_COLLECTION_SIZE + 1)]
        coll.insert_many(docs)
        total_docs += SMALL_COLLECTION_SIZE

    # 4. Generate remaining collections to reach 100+ (80 more x 15 docs each = 1200 docs)
    print(f"\n  Generating {len(DOMAIN_PREFIXES)} additional domain collections...")
    for prefix in DOMAIN_PREFIXES:
        coll_name = f"{prefix}s"  # pluralize
        coll = db.collection(coll_name)
        doc_count = random.randint(10, 25)
        docs = [generate_generic_doc(prefix, i) for i in range(1, doc_count + 1)]
        coll.insert_many(docs)
        total_docs += doc_count

    # Create some indexes on large collections
    print("\n  Creating indexes...")
    db.collection("users").create_index("email", unique=True)
    db.collection("users").create_index("city", unique=False)
    db.collection("users").create_index("status", unique=False)
    db.collection("products").create_index("sku", unique=True)
    db.collection("products").create_index("category", unique=False)
    db.collection("orders").create_index("order_id", unique=True)
    db.collection("orders").create_index("user_id", unique=False)
    db.collection("orders").create_index("status", unique=False)
    db.collection("logs").create_index("level", unique=False)
    db.collection("logs").create_index("service", unique=False)
    db.collection("events").create_index("event_type", unique=False)
    db.collection("events").create_index("user_id", unique=False)

    db.close()

    # Summary
    total_collections = len(large_collections) + len(medium_collections) + len(small_template_collections) + len(DOMAIN_PREFIXES)
    print(f"\n{'='*50}")
    print(f"Database created: {DB_PATH}")
    print(f"Total collections: {total_collections}")
    print(f"Total documents: {total_docs}")
    print(f"Large collections (1500 docs): {len(large_collections)}")
    print(f"Medium collections (200 docs): {len(medium_collections)}")
    print(f"Small collections (10-25 docs): {len(small_template_collections) + len(DOMAIN_PREFIXES)}")

    # File size
    size = os.path.getsize(DB_PATH)
    print(f"File size: {size / 1024 / 1024:.2f} MB")


if __name__ == "__main__":
    main()
