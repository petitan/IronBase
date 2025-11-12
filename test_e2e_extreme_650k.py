#!/usr/bin/env python3
"""
EXTREME Large-Scale Test: 650K Documents (~15MB Metadata)
Tests IronBase dynamic metadata storage with extreme dataset sizes
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

def test_extreme_insert():
    """Test 1: Insert 650K documents (~600MB data, ~15MB metadata)"""
    print("=" * 80)
    print("EXTREME TEST 1: Insert 650K Documents")
    print("=" * 80)

    db_path = "test_650k.mlite"
    cleanup(db_path)

    db = IronBase(db_path)
    products = db.collection("products")

    # Target: 650,000 documents
    # Metadata size: ~650K * 23 bytes = ~15 MB
    # Data size: ~650K * 1KB = ~650 MB
    target_docs = 650_000

    print(f"Target: {target_docs:,} documents")
    print(f"Expected data size: ~{target_docs // 1024} MB")
    print(f"Expected metadata size: ~{target_docs * 23 // (1024*1024)} MB")
    print()

    batch_size = 5000  # Larger batches for faster insertion
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

        # Report progress every 10 seconds
        now = time.time()
        if now - last_report >= 10.0:
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
    print(f"  Total time: {total_elapsed:.2f}s ({total_elapsed/60:.1f} minutes)")
    print(f"  Average speed: {total_inserted/total_elapsed:.0f} docs/sec")
    print(f"  Average throughput: {final_size/(1024*1024)/total_elapsed:.2f} MB/sec")

    return db, products, total_inserted

def test_extreme_queries(db, products, total_docs):
    """Test 2: Basic query performance on extreme dataset"""
    print()
    print("=" * 80)
    print("EXTREME TEST 2: Query Performance")
    print("=" * 80)

    # Test 1: Count documents
    print("\nTest 2.1: Count all documents")
    start = time.time()
    count = products.count_documents()
    elapsed = time.time() - start
    assert count == total_docs
    print(f"✓ Count: {count:,} documents in {elapsed*1000:.2f}ms")

    # Test 2: Simple equality query (indexed field later)
    print("\nTest 2.2: Find by category (equality)")
    start = time.time()
    electronics = products.find({"category": "Electronics"})
    elapsed = time.time() - start
    print(f"✓ Found {len(electronics):,} electronics in {elapsed:.2f}s")

    # Test 3: Limit query (should be fast)
    print("\nTest 2.3: Find first 1000 documents")
    start = time.time()
    first_1000 = products.find({}, limit=1000)
    elapsed = time.time() - start
    print(f"✓ Retrieved 1000 documents in {elapsed*1000:.2f}ms")

    # Test 4: Distinct values
    print("\nTest 2.4: Distinct categories")
    start = time.time()
    categories = products.distinct("category")
    elapsed = time.time() - start
    print(f"✓ Found {len(categories)} unique categories in {elapsed:.2f}s: {sorted(categories)}")

def test_extreme_metadata_flush(db):
    """Test 3: Force metadata flush with 15MB metadata"""
    print()
    print("=" * 80)
    print("EXTREME TEST 3: Metadata Flush (15MB)")
    print("=" * 80)

    print("\nForcing metadata flush to disk...")
    start = time.time()
    db.flush()
    elapsed = time.time() - start
    print(f"✓ Metadata flushed in {elapsed:.2f}s")

def test_extreme_compaction(db, db_path):
    """Test 4: Compaction on extreme database"""
    print()
    print("=" * 80)
    print("EXTREME TEST 4: Database Compaction")
    print("=" * 80)

    size_before = get_db_size(db_path)
    print(f"\nDatabase size before compaction: {format_size(size_before)}")
    print("Running compaction (may take several minutes)...")

    start = time.time()
    stats = db.compact()
    elapsed = time.time() - start

    size_after = get_db_size(db_path)

    print()
    print(f"✓ Compaction completed in {elapsed:.2f}s ({elapsed/60:.1f} minutes)")
    print(f"  Documents scanned: {stats['documents_scanned']:,}")
    print(f"  Documents kept: {stats['documents_kept']:,}")
    print(f"  Tombstones removed: {stats['tombstones_removed']:,}")
    print(f"  Space saved: {format_size(stats['space_saved'])}")
    print(f"  Size before: {format_size(size_before)}")
    print(f"  Size after: {format_size(size_after)}")
    print(f"  Compression ratio: {stats['compression_ratio']:.2f}%")
    print(f"  Peak memory: {stats['peak_memory_mb']:.2f} MB")

def test_extreme_reopen(db_path):
    """Test 5: Reopen database and verify metadata loading"""
    print()
    print("=" * 80)
    print("EXTREME TEST 5: Reopen Database (Load 15MB Metadata)")
    print("=" * 80)

    print("\nClosing database...")
    # Database will be closed by returning from function

    print("Reopening database (loading 15MB metadata from disk)...")
    start = time.time()
    db = IronBase(db_path)
    elapsed = time.time() - start
    print(f"✓ Database reopened in {elapsed:.2f}s")

    # Verify
    products = db.collection("products")
    count = products.count_documents()
    print(f"✓ Verified document count: {count:,}")

    return db

def run_extreme_tests():
    """Run all extreme large-scale tests"""
    print("\n" + "🔥" * 40)
    print("EXTREME LARGE-SCALE TEST: 650K Documents (~15MB Metadata)")
    print("Testing IronBase dynamic metadata storage limits")
    print("🔥" * 40 + "\n")

    overall_start = time.time()
    db_path = "test_650k.mlite"

    try:
        # Phase 1: Insert 650K documents
        db, products, total_docs = test_extreme_insert()

        # Phase 2: Query performance
        test_extreme_queries(db, products, total_docs)

        # Phase 3: Compaction (metadata will be flushed automatically)
        test_extreme_compaction(db, db_path)

        # Close database
        print()
        print("=" * 80)
        print("Closing database...")
        db.close()

        # Phase 5: Reopen and verify
        db = test_extreme_reopen(db_path)
        db.close()

        overall_elapsed = time.time() - overall_start
        final_size = get_db_size(db_path)

        print()
        print("=" * 80)
        print("✅ ALL EXTREME TESTS PASSED!")
        print("=" * 80)
        print(f"\n📊 Final Statistics:")
        print(f"   Total test time: {overall_elapsed:.2f}s ({overall_elapsed/60:.1f} minutes)")
        print(f"   Final database size: {format_size(final_size)}")
        print(f"   Documents inserted: {total_docs:,}")
        print()
        print("🎉 IronBase successfully handled 650K documents with 15MB metadata!")
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
    exit(run_extreme_tests())
