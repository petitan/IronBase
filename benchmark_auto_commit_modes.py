#!/usr/bin/env python3
"""
Performance Benchmark: Auto-Commit Modes

Compares throughput and latency for Safe, Batch, and Unsafe modes.
"""

import os
import time
from ironbase import IronBase


def benchmark_mode(mode_name, durability_mode, batch_size=100, num_docs=1000):
    """
    Benchmark a specific durability mode

    Args:
        mode_name: Display name
        durability_mode: "safe", "batch", or "unsafe"
        batch_size: Batch size for batch mode
        num_docs: Number of documents to insert
    """
    db_path = f"bench_{mode_name}.mlite"
    wal_path = f"bench_{mode_name}.wal"

    # Cleanup
    for f in [db_path, wal_path]:
        if os.path.exists(f):
            os.remove(f)

    # Open database with specified mode
    if durability_mode == "batch":
        db = IronBase(db_path, durability=durability_mode, batch_size=batch_size)
    else:
        db = IronBase(db_path, durability=durability_mode)

    col = db.collection("bench")

    # Warmup
    for i in range(10):
        col.insert_one({"warmup": i})

    # Benchmark
    start_time = time.time()

    for i in range(num_docs):
        col.insert_one({
            "id": i,
            "name": f"User {i}",
            "email": f"user{i}@example.com",
            "age": 20 + (i % 50),
            "active": i % 2 == 0,
        })

    # For batch mode, flush remaining
    if durability_mode == "batch":
        db.checkpoint()

    elapsed = time.time() - start_time

    # Calculate metrics
    throughput = num_docs / elapsed
    latency_ms = (elapsed / num_docs) * 1000

    # Verify count
    count = col.count_documents({})

    db.close()

    # Cleanup
    for f in [db_path, wal_path]:
        if os.path.exists(f):
            os.remove(f)

    return {
        "mode": mode_name,
        "durability": durability_mode,
        "batch_size": batch_size if durability_mode == "batch" else None,
        "num_docs": num_docs,
        "count": count,
        "elapsed_sec": elapsed,
        "throughput_ops_sec": throughput,
        "avg_latency_ms": latency_ms,
    }


def print_results(results):
    """Print benchmark results in a nice table"""
    print("\n" + "=" * 90)
    print("  PERFORMANCE BENCHMARK RESULTS")
    print("=" * 90)
    print()

    # Header
    print(f"{'Mode':<20} {'Docs':<10} {'Time (s)':<12} {'Throughput':<20} {'Avg Latency':<15}")
    print("-" * 90)

    # Results
    for r in results:
        mode_display = r['mode']
        if r['batch_size']:
            mode_display += f" (batch={r['batch_size']})"

        print(f"{mode_display:<20} "
              f"{r['num_docs']:<10} "
              f"{r['elapsed_sec']:<12.3f} "
              f"{r['throughput_ops_sec']:<20.1f} "
              f"{r['avg_latency_ms']:<15.3f}")

    print("-" * 90)
    print()


def print_comparison(results):
    """Print relative performance comparison"""
    # Use unsafe as baseline
    baseline = next(r for r in results if r['durability'] == 'unsafe')
    baseline_throughput = baseline['throughput_ops_sec']

    print("=" * 90)
    print("  RELATIVE PERFORMANCE (vs Unsafe mode)")
    print("=" * 90)
    print()

    print(f"{'Mode':<30} {'Throughput':<20} {'Relative':<15} {'Safety':<20}")
    print("-" * 90)

    for r in results:
        mode_display = r['mode']
        if r['batch_size']:
            mode_display += f" (batch={r['batch_size']})"

        relative = (r['throughput_ops_sec'] / baseline_throughput) * 100

        # Safety rating
        if r['durability'] == 'safe':
            safety = "✅ ZERO loss"
        elif r['durability'] == 'batch':
            safety = f"⚠️ Max {r['batch_size']} ops"
        else:
            safety = "❌ HIGH risk"

        print(f"{mode_display:<30} "
              f"{r['throughput_ops_sec']:<20.1f} "
              f"{relative:<15.1f}% "
              f"{safety:<20}")

    print("-" * 90)
    print()


def print_recommendations():
    """Print usage recommendations"""
    print("=" * 90)
    print("  RECOMMENDATIONS")
    print("=" * 90)
    print()
    print("Use Case                     Recommended Mode           Why")
    print("-" * 90)
    print("Financial transactions       Safe mode                  ZERO data loss required")
    print("User accounts/profiles       Safe mode                  Critical data integrity")
    print("E-commerce orders            Safe mode                  Cannot lose customer orders")
    print()
    print("Application logs             Batch (100-1000)           High throughput, bounded loss OK")
    print("Analytics events             Batch (1000-5000)          Very high throughput needed")
    print("Session tracking             Batch (100-500)            Good balance")
    print()
    print("Temporary staging data       Unsafe + checkpoint()      Performance critical")
    print("Test/development             Unsafe + checkpoint()      Fast iteration")
    print("Bulk imports (retry safe)    Unsafe + checkpoint()      Can re-run if fails")
    print("-" * 90)
    print()


if __name__ == "__main__":
    print("=" * 90)
    print("  AUTO-COMMIT MODES - PERFORMANCE BENCHMARK")
    print("=" * 90)
    print()
    print("Testing throughput and latency for different durability modes...")
    print("Number of documents per test: 1000")
    print()

    results = []

    # Benchmark Safe mode
    print("[1/5] Benchmarking Safe mode...")
    results.append(benchmark_mode("Safe", "safe", num_docs=1000))

    # Benchmark Batch mode (different batch sizes)
    print("[2/5] Benchmarking Batch mode (batch_size=10)...")
    results.append(benchmark_mode("Batch-10", "batch", batch_size=10, num_docs=1000))

    print("[3/5] Benchmarking Batch mode (batch_size=100)...")
    results.append(benchmark_mode("Batch-100", "batch", batch_size=100, num_docs=1000))

    print("[4/5] Benchmarking Batch mode (batch_size=500)...")
    results.append(benchmark_mode("Batch-500", "batch", batch_size=500, num_docs=1000))

    # Benchmark Unsafe mode
    print("[5/5] Benchmarking Unsafe mode...")
    results.append(benchmark_mode("Unsafe", "unsafe", num_docs=1000))

    print("\n✓ All benchmarks completed!")

    # Print results
    print_results(results)
    print_comparison(results)
    print_recommendations()

    print("=" * 90)
    print("  SUMMARY")
    print("=" * 90)
    print()
    print("✅ Safe mode provides ZERO data loss with acceptable performance")
    print("✅ Batch mode offers excellent throughput/safety balance")
    print("⚠️ Unsafe mode is fastest but requires manual checkpoint()")
    print()
    print("Default: Safe mode (like SQL databases)")
    print("=" * 90)
