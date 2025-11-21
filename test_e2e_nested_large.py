#!/usr/bin/env python3
"""Large-scale nested document E2E test for IronBase."""

import argparse
import os
import random
import string
import time
from datetime import datetime, timedelta

from ironbase import IronBase

IronBase.set_log_level("WARN")


def cleanup(path: str):
    for ext in (".mlite", ".wal"):
        target = path.replace(".mlite", ext)
        if os.path.exists(target):
            os.remove(target)


def format_size(num_bytes: int) -> str:
    units = ["B", "KB", "MB", "GB"]
    value = float(num_bytes)
    for unit in units:
        if value < 1024.0:
            return f"{value:.2f} {unit}"
        value /= 1024.0
    return f"{value:.2f} TB"


def get_db_size(path: str) -> int:
    return os.path.getsize(path) if os.path.exists(path) else 0


def rand_string(length: int) -> str:
    return "".join(random.choices(string.ascii_letters + string.digits, k=length))


def rand_datetime() -> str:
    base = datetime(2025, random.randint(1, 12), random.randint(1, 28), random.randint(0, 23), random.randint(0, 59))
    delta = timedelta(minutes=random.randint(0, 10_000))
    return (base - delta).isoformat()


def generate_user(doc_id: int) -> dict:
    cities = [
        ("Budapest", "Hungary"),
        ("Debrecen", "Hungary"),
        ("Prague", "Czech Republic"),
        ("Vienna", "Austria"),
        ("Warsaw", "Poland"),
        ("Berlin", "Germany"),
    ]
    city, country = random.choice(cities)
    profile = {
        "name": f"User {rand_string(8)}",
        "age": random.randint(18, 75),
        "location": {
            "city": city,
            "country": country,
            "geo": {
                "lat": round(random.uniform(-90, 90), 5),
                "lng": round(random.uniform(-180, 180), 5),
            },
            "address": {
                "street": f"{random.randint(1, 200)} {rand_string(6)} St",
                "zip": random.randint(1000, 99999),
            },
        },
        "contacts": {
            "email": f"user{doc_id}@example.com",
            "phones": [f"+36{random.randint(100000000, 999999999)}" for _ in range(random.randint(1, 2))],
        },
    }
    preferences = {
        "notifications": {
            "email": random.choice([True, False]),
            "sms": random.choice([True, False]),
            "in_app": True,
        },
        "themes": {
            "mode": random.choice(["light", "dark", "amoled"]),
            "color": random.choice(["blue", "green", "purple", "orange"]),
        },
    }
    metrics = {
        "login": {
            "count": random.randint(0, 5000),
            "last_login": rand_datetime(),
        },
        "orders": {
            "total_value": round(random.uniform(0, 20000), 2),
            "last_order_value": round(random.uniform(5, 1000), 2),
        },
    }
    sessions = [
        {
            "device": random.choice(["ios", "android", "web", "desktop"]),
            "location": {
                "city": city,
                "ip": f"192.168.{random.randint(0, 255)}.{random.randint(0, 255)}",
            },
            "traits": {
                "browser": random.choice(["chrome", "firefox", "safari", "edge"]),
                "version": f"{random.randint(70, 120)}.0",
            },
        }
        for _ in range(3)
    ]
    return {
        "user_id": doc_id,
        "profile": profile,
        "preferences": preferences,
        "metrics": metrics,
        "sessions": sessions,
        "tags": [rand_string(6) for _ in range(random.randint(3, 7))],
    }


def stage_insert(db_path: str, target_docs: int, batch_size: int):
    print("=" * 80)
    print("NESTED TEST 1: Massive nested insert")
    print("=" * 80)

    cleanup(db_path)
    db = IronBase(db_path)
    users = db.collection("users")

    total = 0
    start = time.time()
    last_report = start

    print(f"Target documents: {target_docs:,}")
    while total < target_docs:
        batch = [generate_user(total + i) for i in range(min(batch_size, target_docs - total))]
        result = users.insert_many(batch)
        total += result["inserted_count"]
        now = time.time()
        if now - last_report >= 5:
            db_size = get_db_size(db_path)
            elapsed = now - start
            speed = total / elapsed if elapsed > 0 else 0
            print(f"  Progress: {total:,}/{target_docs:,} docs | Size: {format_size(db_size)} | Speed: {speed:.0f} docs/sec")
            last_report = now

    elapsed = time.time() - start
    size = get_db_size(db_path)
    print(f"✓ Insert complete: {total:,} docs in {elapsed:.2f}s ({format_size(size)})")

    return db, users, total


def stage_queries(users, total_docs):
    print()
    print("=" * 80)
    print("NESTED TEST 2: Query + analytics")
    print("=" * 80)

    start = time.time()
    count_hu = users.count_documents({"profile.location.country": "Hungary"})
    print(f"✓ Hungarians: {count_hu:,} docs in {(time.time() - start)*1000:.2f} ms")

    start = time.time()
    count_budapest = users.count_documents({"profile.location.city": "Budapest"})
    elapsed = (time.time() - start) * 1000
    print(f"✓ Budapest residents: {count_budapest:,} docs in {elapsed:.2f} ms")
    samples = users.find({"profile.location.city": "Budapest"}, limit=3)
    print(f"  Sample names: {[doc['profile']['name'] for doc in samples]}")

    start = time.time()
    count_heavy = users.count_documents({"metrics.login.count": {"$gte": 1000}})
    elapsed = (time.time() - start) * 1000
    print(f"✓ Heavy login users (>=1000): {count_heavy:,} docs in {elapsed:.2f} ms")
    heavy_sample = users.find({"metrics.login.count": {"$gte": 1000}}, limit=3)
    print(f"  Sample login counts: {[doc['metrics']['login']['count'] for doc in heavy_sample]}")

    start = time.time()
    engaged = users.find({
        "$and": [
            {"preferences.notifications.email": True},
            {"metrics.orders.total_value": {"$gte": 1000}}
        ]
    }, limit=10)
    print(f"✓ Email opt-in & >1k orders: showing {len(engaged)} of many in {(time.time() - start)*1000:.2f} ms")

    print("\nCreating nested indexes for plan analysis...")
    idx_city = users.create_index("profile.location.city")
    idx_login = users.create_index("metrics.login.count")
    idx_pref = users.create_index("preferences.notifications.email")
    print(f"  ✓ Indexes: {idx_city}, {idx_login}, {idx_pref}")

    explain = users.explain({"profile.location.city": "Budapest"})
    print(f"✓ Query planner for city lookup: {explain}")
    return {
        "city": idx_city,
        "login": idx_login,
        "pref": idx_pref,
    }


def stage_updates(users):
    print()
    print("=" * 80)
    print("NESTED TEST 3: Updates & deletes")
    print("=" * 80)

    start = time.time()
    update = users.update_many({"profile.location.country": "Hungary"}, {"$set": {"metrics.segment": "hu-loyal"}})
    print(f"✓ Update many (HU segment): matched {update['matched_count']} in {(time.time() - start)*1000:.2f} ms")

    start = time.time()
    removal = users.delete_many({
        "profile.location.city": "Debrecen",
        "metrics.login.count": {"$lt": 10}
    })
    print(f"✓ Removed {removal.get('deleted_count', 0)} cold Debrecen users in {(time.time() - start)*1000:.2f} ms")


def stage_compaction(db, db_path: str):
    print()
    print("=" * 80)
    print("NESTED TEST 4: Compaction & closing")
    print("=" * 80)

    before = get_db_size(db_path)
    start = time.time()
    stats = db.compact()
    elapsed = time.time() - start
    after = get_db_size(db_path)

    print(f"✓ Compaction finished in {elapsed:.2f}s | {format_size(before)} -> {format_size(after)}")
    print(f"  Tombstones removed: {stats['tombstones_removed']}, documents scanned: {stats['documents_scanned']}")


def parse_args():
    parser = argparse.ArgumentParser(description="Nested large-scale E2E test")
    parser.add_argument("--db", default="test_nested_large.mlite", help="Database path")
    parser.add_argument("--target-docs", type=int, default=52_428, help="Number of documents to insert")
    parser.add_argument("--batch-size", type=int, default=1_500, help="Insert batch size")
    return parser.parse_args()


def main():
    args = parse_args()
    print("#" * 90)
    print(" NESTED LARGE-SCALE E2E TEST ")
    print("#" * 90)

    db, users, total_docs = stage_insert(args.db, args.target_docs, args.batch_size)
    try:
        indexes = stage_queries(users, total_docs)
        stage_updates(users)
        stage_compaction(db, args.db)
        print("\nSUMMARY:")
        print(f"  Documents inserted: {total_docs:,}")
        remaining = users.count_documents({})
        print(f"  Remaining docs after cleanup: {remaining:,}")
    finally:
        db.close()
        cleanup(args.db)
        print("\n✓ Database closed and files cleaned up")


if __name__ == "__main__":
    main()
