#!/usr/bin/env python3
"""
IronBase Comprehensive Test Runner
Runs all test suites and generates a summary report
"""
import subprocess
import sys
import os
from pathlib import Path

# Define test suites in execution order
TEST_SUITES = [
    # Core CRUD operations
    ("test_mongolite.py", "Find operations"),
    ("test_update.py", "Update operations"),
    ("test_delete.py", "Delete operations"),
    ("test_count.py", "Count operations"),

    # Advanced features
    ("test_complex_queries.py", "Complex queries"),
    ("test_transactions.py", "Transactions"),
    ("test_indexes.py", "Indexes"),
    ("test_distinct.py", "Distinct operation"),
    ("test_find_options.py", "Find options (limit, skip, sort)"),

    # Compaction and persistence
    ("test_compaction_simple.py", "Compaction (simple)"),
    ("test_compaction.py", "Compaction (comprehensive)"),
    ("test_reopen_fixed.py", "Database reopen/persistence"),

    # Index persistence
    ("test_index_persistence_poc.py", "B+ tree persistence (Rust unit tests)"),
]

def run_test(test_file, description):
    """Run a single test file and return results"""
    print(f"\n{'=' * 70}")
    print(f"Running: {description}")
    print(f"File: {test_file}")
    print('=' * 70)

    try:
        result = subprocess.run(
            ["python", test_file],
            capture_output=True,
            text=True,
            timeout=60
        )

        if result.returncode == 0:
            print(f"✅ PASSED")
            # Show last few lines of output
            lines = result.stdout.strip().split('\n')
            for line in lines[-5:]:
                print(f"   {line}")
            return True
        else:
            print(f"❌ FAILED")
            print(f"\nSTDOUT:")
            print(result.stdout)
            print(f"\nSTDERR:")
            print(result.stderr)
            return False

    except subprocess.TimeoutExpired:
        print(f"⏱️ TIMEOUT (> 60s)")
        return False
    except Exception as e:
        print(f"💥 ERROR: {e}")
        return False

def main():
    print("=" * 70)
    print("IronBase Comprehensive Test Suite")
    print("=" * 70)

    results = []
    passed = 0
    failed = 0

    for test_file, description in TEST_SUITES:
        if not os.path.exists(test_file):
            print(f"\n⚠️ SKIPPED: {test_file} (file not found)")
            continue

        success = run_test(test_file, description)
        results.append((test_file, description, success))

        if success:
            passed += 1
        else:
            failed += 1

    # Summary report
    print("\n" + "=" * 70)
    print("TEST SUMMARY")
    print("=" * 70)

    for test_file, description, success in results:
        status = "✅ PASS" if success else "❌ FAIL"
        print(f"{status} - {description}")

    print("\n" + "=" * 70)
    print(f"Total: {passed + failed} tests")
    print(f"Passed: {passed} ✅")
    print(f"Failed: {failed} ❌")

    if failed == 0:
        print("\n🎉 ALL TESTS PASSED!")
        print("=" * 70)
        return 0
    else:
        print(f"\n⚠️ {failed} test(s) failed")
        print("=" * 70)
        return 1

if __name__ == "__main__":
    sys.exit(main())
