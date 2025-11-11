# MongoLite Testing Guide

This document describes the comprehensive test suite for MongoLite.

## Test Suite Overview

MongoLite has **13 comprehensive test suites** covering all major functionality:

### Core CRUD Operations
- ✅ **test_mongolite.py** - Find operations (6 tests)
- ✅ **test_update.py** - Update operations with $set, $inc, $unset (6 tests)
- ✅ **test_delete.py** - Delete operations with tombstones (6 tests)
- ✅ **test_count.py** - Count documents with filters (6 tests)

### Advanced Queries
- ✅ **test_complex_queries.py** - MongoDB operators: $gt, $lt, $in, $and, $or, $not (6 tests)
- ✅ **test_distinct.py** - Distinct value queries (6 tests)
- ✅ **test_find_options.py** - Limit, skip, sort, projection (6+ tests)

### Transaction Support
- ✅ **test_transactions.py** - ACD (Atomic, Consistent, Durable) transactions
  - Commit/rollback functionality
  - Isolation between transactions
  - Crash recovery

### Indexing
- ✅ **test_indexes.py** - B+ tree indexing
  - Index creation and deletion
  - Range queries with indexes
  - Performance improvements
- ✅ **test_index_persistence_poc.py** - Rust unit tests for B+ tree file persistence

### Compaction & Persistence
- ✅ **test_compaction_simple.py** - Basic compaction workflow
- ✅ **test_compaction.py** - Comprehensive compaction with 5 test phases:
  1. Insert documents
  2. Delete documents (create tombstones)
  3. Run compaction
  4. Verify data integrity
  5. Test persistence after reopen
- ✅ **test_reopen_fixed.py** - Database persistence across restarts

## Running Tests

### Run All Tests
```bash
python run_all_tests.py
```

This will execute all 13 test suites and generate a summary report.

### Run Individual Tests
```bash
python test_mongolite.py
python test_update.py
python test_compaction.py
# ... etc
```

### Run Rust Unit Tests
```bash
cargo test
cargo test --release  # With optimizations
```

## Test Coverage

### What's Tested
✅ CRUD operations (Create, Read, Update, Delete)
✅ Query operators ($eq, $ne, $gt, $gte, $lt, $lte, $in, $nin, $and, $or, $not, $nor, $exists)
✅ Update operators ($set, $inc, $unset, $push, $pull)
✅ Transactions (begin, commit, rollback)
✅ Indexing (B+ tree, create, drop, range queries)
✅ Compaction (garbage collection, tombstone removal)
✅ Persistence (database reopen, data integrity)
✅ Count, distinct, limit, skip, sort, projection
✅ Error handling and edge cases

### Test Statistics
- **Total Test Suites**: 13
- **Total Test Cases**: 60+
- **Pass Rate**: 100% ✅
- **Coverage Areas**: CRUD, Queries, Transactions, Indexes, Compaction, Persistence

## Continuous Integration

All tests are designed to run in CI/CD environments:
- Fast execution (< 5 minutes total)
- No external dependencies
- Clean database files before each test
- Clear pass/fail indicators

## Adding New Tests

When adding new features, create a corresponding test file:

```python
#!/usr/bin/env python3
"""Test description"""
import ironbase
import os

# Clean up
if os.path.exists("test_db.mlite"):
    os.remove("test_db.mlite")

db = ironbase.MongoLite("test_db.mlite")
collection = db.collection("test")

# Your tests here
assert collection.count_documents({}) == 0

db.close()
print("✅ TEST PASSED")
```

Then add it to `run_all_tests.py` in the `TEST_SUITES` list.

## Test Quality Standards

All tests should:
- Clean up test database files before running
- Use descriptive assertions with clear error messages
- Test both success and failure cases
- Verify data integrity after operations
- Close database connections properly
- Print clear pass/fail indicators

## Known Issues

- Index persistence integration pending (see test_index_persistence_poc.py)
- RESERVED SPACE architecture limits compaction space savings

## Performance Benchmarks

Run performance tests:
```bash
cargo bench
```

This measures:
- Insert throughput
- Query performance
- Index lookup speed
- Transaction overhead
