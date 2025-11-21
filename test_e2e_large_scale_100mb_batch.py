#!/usr/bin/env python3
"""
Large-Scale E2E Test: 100MB Dataset
Tests IronBase performance and stability with realistic large datasets
"""

from ironbase import IronBase
import os
import time
import random
import string

# Disable verbose logging for performance
IronBase.set_log_level("WARN")

def cleanup(path):
    """Clean up test database files"""
    for ext in [".mlite", ".wal"]:
        try:
            os.remove(path.replace(".mlite", ext))
        except FileNotFoundError:
            pass

def format_size(bytes_size):
    """Format bytes to human readable size"""
    for unit in ['B', 'KB', 'MB', 'GB']:
        if bytes_size < 1024.0:
            return f"{bytes_size:.2f} {unit}"
        bytes_size /= 1024.0
    return f"{bytes_size:.2f} TB"

def get_db_size(db_path):
    """Get database file size"""
    try:
        size = os.path.getsize(db_path)
        return size
    except FileNotFoundError:
        return 0

def generate_random_string(length):
    """Generate random string of given length"""
    return ''.join(random.choices(string.ascii_letters + string.digits, k=length))

def generate_product_document():
    """Generate a realistic product document (~1KB each)"""
    categories = ["Electronics", "Books", "Clothing", "Home & Garden", "Sports", "Toys", "Food"]
    brands = ["BrandA", "BrandB", "BrandC", "BrandD", "BrandE"]

    return {
        "product_id": generate_random_string(10),
        "name": f"Product {generate_random_string(20)}",
        "category": random.choice(categories),
        "brand": random.choice(brands),
        "price": round(random.uniform(5.99, 999.99), 2),
        "stock": random.randint(0, 1000),
        "description": generate_random_string(200),  # 200 chars
        "features": [generate_random_string(50) for _ in range(5)],  # 5 features
        "reviews_count": random.randint(0, 10000),
        "rating": round(random.uniform(1.0, 5.0), 1),
        "tags": [generate_random_string(10) for _ in range(random.randint(3, 8))],
        "metadata": {
            "created_at": f"2025-{random.randint(1,12):02d}-{random.randint(1,28):02d}",
            "updated_at": f"2025-{random.randint(1,12):02d}-{random.randint(1,28):02d}",
            "warehouse": f"WH-{random.randint(1,10)}",
            "supplier_id": generate_random_string(15)
        }
    }

def test_large_scale_insert():
    """Test 1: Insert ~100MB of data"""
    print("=" * 80)
    print("LARGE-SCALE TEST 1: Insert 100MB Dataset")
    print("=" * 80)

    db_path = "test_100mb.mlite"
    cleanup(db_path)

    db = IronBase(db_path, durability="batch", batch_size=500)
    products = db.collection("products")

    # Calculate: each document ~1KB, so need ~100,000 documents for 100MB
    target_size_mb = 100
    estimated_doc_size = 1024  # 1KB per document
    target_docs = (target_size_mb * 1024 * 1024) // estimated_doc_size

    print(f"Target: ~{target_size_mb} MB")
    print(f"Estimated documents needed: {target_docs:,}")
    print()

    batch_size = 1000
    total_inserted = 0
    start_time = time.time()
    last_report = start_time

    print("Inserting data...")
    while total_inserted < target_docs:
        # Generate batch
        batch = [generate_product_document() for _ in range(batch_size)]

        # Insert batch
        batch_start = time.time()
        result = products.insert_many(batch)
        batch_elapsed = time.time() - batch_start

        total_inserted += result["inserted_count"]

        # Report progress every 5 seconds
        now = time.time()
        if now - last_report >= 5.0:
            elapsed = now - start_time
            db_size = get_db_size(db_path)
            docs_per_sec = total_inserted / elapsed if elapsed > 0 else 0

            print(f"  Progress: {total_inserted:,}/{target_docs:,} docs "
                  f"({total_inserted/target_docs*100:.1f}%) | "
                  f"DB size: {format_size(db_size)} | "
                  f"Speed: {docs_per_sec:.0f} docs/sec | "
                  f"Batch: {batch_elapsed*1000:.1f}ms")
            last_report = now

    total_elapsed = time.time() - start_time
    final_size = get_db_size(db_path)

    print()
    print(f"✓ Insert completed!")
    print(f"  Total documents: {total_inserted:,}")
    print(f"  Database size: {format_size(final_size)}")
    print(f"  Total time: {total_elapsed:.2f}s")
    print(f"  Average speed: {total_inserted/total_elapsed:.0f} docs/sec")
    print(f"  Average throughput: {final_size/(1024*1024)/total_elapsed:.2f} MB/sec")

    return db, products, total_inserted

def test_large_scale_queries(db, products, total_docs):
    """Test 2: Query performance on large dataset"""
    print()
    print("=" * 80)
    print("LARGE-SCALE TEST 2: Query Performance")
    print("=" * 80)

    # Test 1: Count documents
    print("\nTest 2.1: Count all documents")
    start = time.time()
    count = products.count_documents()
    elapsed = time.time() - start
    assert count == total_docs
    print(f"✓ Count: {count:,} documents in {elapsed*1000:.2f}ms")

    # Test 2: Simple equality query
    print("\nTest 2.2: Find by category (equality)")
    start = time.time()
    electronics = products.find({"category": "Electronics"})
    elapsed = time.time() - start
    print(f"✓ Found {len(electronics):,} electronics in {elapsed*1000:.2f}ms")

    # Test 3: Range query
    print("\nTest 2.3: Find by price range")
    start = time.time()
    mid_price = products.find({"price": {"$gte": 100, "$lte": 500}})
    elapsed = time.time() - start
    print(f"✓ Found {len(mid_price):,} products in $100-500 range in {elapsed*1000:.2f}ms")

    # Test 4: Sort and limit
    print("\nTest 2.4: Sort by price (top 100)")
    start = time.time()
    expensive = products.find({}, sort=[("price", -1)], limit=100)
    elapsed = time.time() - start
    print(f"✓ Retrieved top 100 expensive products in {elapsed*1000:.2f}ms")
    print(f"  Highest price: ${expensive[0]['price']:.2f}")

    # Test 5: Projection query
    print("\nTest 2.5: Projection (name and price only, 1000 docs)")
    start = time.time()
    projected = products.find({}, projection={"name": 1, "price": 1}, limit=1000)
    elapsed = time.time() - start
    print(f"✓ Retrieved 1000 projected docs in {elapsed*1000:.2f}ms")

    # Test 6: Distinct values
    print("\nTest 2.6: Distinct categories")
    start = time.time()
    categories = products.distinct("category")
    elapsed = time.time() - start
    print(f"✓ Found {len(categories)} unique categories in {elapsed*1000:.2f}ms: {sorted(categories)}")

    # Test 7: Aggregation
    print("\nTest 2.7: Aggregation (group by category, count)")
    start = time.time()
    pipeline = [
        {"$group": {"_id": "$category", "count": {"$sum": 1}, "avg_price": {"$avg": "$price"}}},
        {"$sort": {"count": -1}}
    ]
    results = products.aggregate(pipeline)
    elapsed = time.time() - start
    print(f"✓ Aggregation completed in {elapsed*1000:.2f}ms")
    for r in results:
        print(f"  {r['_id']}: {r['count']:,} products, avg price ${r.get('avg_price', 0):.2f}")

def test_large_scale_updates(db, products, total_docs):
    """Test 3: Update operations on large dataset"""
    print()
    print("=" * 80)
    print("LARGE-SCALE TEST 3: Update Operations")
    print("=" * 80)

    # Test 1: Update single document
    print("\nTest 3.1: Update one document")
    start = time.time()
    result = products.update_one(
        {"category": "Electronics"},
        {"$set": {"featured": True, "discount": 15}}
    )
    elapsed = time.time() - start
    print(f"✓ Updated {result['modified_count']} document in {elapsed*1000:.2f}ms")

    # Test 2: Update many documents (small batch)
    print("\nTest 3.2: Update many (price increase for Books)")
    start = time.time()
    result = products.update_many(
        {"category": "Books"},
        {"$inc": {"price": 5.0}}
    )
    elapsed = time.time() - start
    print(f"✓ Updated {result['modified_count']:,} documents in {elapsed*1000:.2f}ms")
    print(f"  Speed: {result['modified_count']/elapsed:.0f} docs/sec")

def test_large_scale_deletes(db, products, total_docs):
    """Test 4: Delete operations on large dataset"""
    print()
    print("=" * 80)
    print("LARGE-SCALE TEST 4: Delete Operations")
    print("=" * 80)

    # Test 1: Delete single document
    print("\nTest 4.1: Delete one document")
    start = time.time()
    result = products.delete_one({"category": "Toys"})
    elapsed = time.time() - start
    print(f"✓ Deleted {result['deleted_count']} document in {elapsed*1000:.2f}ms")

    # Test 2: Delete many documents (10% of database)
    delete_count_target = total_docs // 10
    print(f"\nTest 4.2: Delete many (~10% of database, ~{delete_count_target:,} docs)")
    print("  (Deleting products with stock = 0)")

    start = time.time()
    result = products.delete_many({"stock": 0})
    elapsed = time.time() - start

    print(f"✓ Deleted {result['deleted_count']:,} documents in {elapsed:.2f}s")
    print(f"  Speed: {result['deleted_count']/elapsed:.0f} docs/sec")

    # Verify count
    new_count = products.count_documents()
    print(f"  Remaining documents: {new_count:,}")

    return new_count

def test_large_scale_indexing(db, products):
    """Test 5: Indexing on large dataset"""
    print()
    print("=" * 80)
    print("LARGE-SCALE TEST 5: Indexing")
    print("=" * 80)

    # Test 1: Create index
    print("\nTest 5.1: Create index on 'category' field")
    start = time.time()
    idx_name = products.create_index("category", unique=False)
    elapsed = time.time() - start
    print(f"✓ Index created: {idx_name} in {elapsed:.2f}s")

    # Test 2: Query with index hint
    print("\nTest 5.2: Query with index hint")
    start = time.time()
    results = products.find_with_hint({"category": "Electronics"}, idx_name)
    elapsed = time.time() - start
    print(f"✓ Found {len(results):,} documents using index in {elapsed*1000:.2f}ms")

    # Test 3: Explain query
    print("\nTest 5.3: Explain query plan")
    explanation = products.explain({"category": "Electronics"})
    print(f"✓ Query plan: {explanation.get('queryPlan', 'N/A')}")
    print(f"  Index used: {explanation.get('indexUsed', 'N/A')}")
    print(f"  Estimated cost: {explanation.get('estimatedCost', 'N/A')}")

def test_large_scale_compaction(db, db_path, remaining_docs):
    """Test 6: Compaction on large database"""
    print()
    print("=" * 80)
    print("LARGE-SCALE TEST 6: Database Compaction")
    print("=" * 80)

    size_before = get_db_size(db_path)
    print(f"\nDatabase size before compaction: {format_size(size_before)}")
    print("Running compaction (may take 30-60 seconds)...")

    start = time.time()
    stats = db.compact()
    elapsed = time.time() - start

    size_after = get_db_size(db_path)

    print()
    print(f"✓ Compaction completed in {elapsed:.2f}s")
    print(f"  Documents scanned: {stats['documents_scanned']:,}")
    print(f"  Documents kept: {stats['documents_kept']:,}")
    print(f"  Tombstones removed: {stats['tombstones_removed']:,}")
    print(f"  Space saved: {format_size(stats['space_saved'])}")
    print(f"  Size before: {format_size(size_before)}")
    print(f"  Size after: {format_size(size_after)}")
    print(f"  Compression ratio: {stats['compression_ratio']:.2f}%")
    print(f"  Peak memory: {stats['peak_memory_mb']:.2f} MB")

def run_large_scale_tests():
    """Run all large-scale E2E tests"""
    print("\n" + "🔥" * 40)
    print("LARGE-SCALE E2E TEST: 100MB Dataset")
    print("Testing IronBase with realistic large data")
    print("🔥" * 40 + "\n")

    overall_start = time.time()
    db_path = "test_100mb.mlite"

    try:
        # Phase 1: Insert 100MB data
        db, products, total_docs = test_large_scale_insert()

        # Phase 2: Query performance
        test_large_scale_queries(db, products, total_docs)

        # Phase 3: Update operations
        test_large_scale_updates(db, products, total_docs)

        # Phase 4: Delete operations
        remaining_docs = test_large_scale_deletes(db, products, total_docs)

        # Phase 5: Indexing
        test_large_scale_indexing(db, products)

        # Phase 6: Compaction
        test_large_scale_compaction(db, db_path, remaining_docs)

        # Close database
        print()
        print("=" * 80)
        print("Closing database...")
        db.close()

        overall_elapsed = time.time() - overall_start
        final_size = get_db_size(db_path)

        print()
        print("=" * 80)
        print("✅ ALL LARGE-SCALE TESTS PASSED!")
        print("=" * 80)
        print(f"\n📊 Final Statistics:")
        print(f"   Total test time: {overall_elapsed:.2f}s ({overall_elapsed/60:.1f} minutes)")
        print(f"   Final database size: {format_size(final_size)}")
        print(f"   Documents inserted: {total_docs:,}")
        print(f"   Documents remaining: {remaining_docs:,}")
        print()
        print("🎉 IronBase successfully handled 100MB+ dataset!")
        print()

        # Cleanup
        print("Cleaning up test files...")
        cleanup(db_path)
        print("✓ Cleanup complete")

        return 0

    except Exception as e:
        print(f"\n❌ TEST FAILED: {e}")
        import traceback
        traceback.print_exc()

        # Cleanup on error
        try:
            cleanup(db_path)
        except:
            pass

        return 1

if __name__ == "__main__":
    exit(run_large_scale_tests())
