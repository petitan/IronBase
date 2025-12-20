// chaos_concurrent_tests.rs
// Phase 3: Concurrent Stress & Race Condition Tests
//
// These tests verify thread safety under heavy concurrent load:
// 1. No deadlocks occur
// 2. Data integrity is maintained
// 3. All operations complete without panic

use ironbase_core::database::DatabaseCore;
use ironbase_core::storage::MemoryStorage;
use serde_json::json;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Barrier};
use std::thread;
use std::time::Duration;
use tempfile::TempDir;

// =============================================================================
// CONCURRENT INSERT TESTS
// =============================================================================

/// Test: Many threads inserting simultaneously
/// Expected: All documents inserted, no panics, correct count
#[test]
fn test_concurrent_inserts_memory() {
    const NUM_THREADS: usize = 10;
    const DOCS_PER_THREAD: usize = 100;

    let db = Arc::new(DatabaseCore::<MemoryStorage>::open_memory().unwrap());
    db.collection("stress").unwrap();

    let barrier = Arc::new(Barrier::new(NUM_THREADS));

    let handles: Vec<_> = (0..NUM_THREADS)
        .map(|thread_id| {
            let db = Arc::clone(&db);
            let barrier = Arc::clone(&barrier);

            thread::spawn(move || {
                barrier.wait(); // All threads start together

                for i in 0..DOCS_PER_THREAD {
                    let doc = HashMap::from([
                        ("thread".to_string(), json!(thread_id)),
                        ("seq".to_string(), json!(i)),
                        ("data".to_string(), json!(format!("t{}_{}", thread_id, i))),
                    ]);
                    db.insert_one("stress", doc).expect("Insert should succeed");
                }
            })
        })
        .collect();

    // Wait for all threads
    for handle in handles {
        handle.join().expect("Thread should not panic");
    }

    // Verify count
    let collection = db.collection("stress").unwrap();
    let count = collection.count_documents(&json!({})).unwrap();
    assert_eq!(
        count,
        (NUM_THREADS * DOCS_PER_THREAD) as u64,
        "All documents should be inserted"
    );
}

/// Test: Concurrent inserts with file-based storage
#[test]
fn test_concurrent_inserts_file() {
    const NUM_THREADS: usize = 5;
    const DOCS_PER_THREAD: usize = 50;

    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("concurrent.mlite");

    let db = Arc::new(DatabaseCore::open(&db_path).unwrap());
    db.collection("stress").unwrap();

    let barrier = Arc::new(Barrier::new(NUM_THREADS));

    let handles: Vec<_> = (0..NUM_THREADS)
        .map(|thread_id| {
            let db = Arc::clone(&db);
            let barrier = Arc::clone(&barrier);

            thread::spawn(move || {
                barrier.wait();

                for i in 0..DOCS_PER_THREAD {
                    let doc = HashMap::from([
                        ("thread".to_string(), json!(thread_id)),
                        ("seq".to_string(), json!(i)),
                    ]);
                    db.insert_one("stress", doc).expect("Insert should succeed");
                }
            })
        })
        .collect();

    for handle in handles {
        handle.join().expect("Thread should not panic");
    }

    let collection = db.collection("stress").unwrap();
    let count = collection.count_documents(&json!({})).unwrap();
    assert_eq!(count, (NUM_THREADS * DOCS_PER_THREAD) as u64);
}

// =============================================================================
// READ/WRITE CONCURRENCY TESTS
// =============================================================================

/// Test: Concurrent reads during writes
/// Expected: Readers see consistent data, no panics
#[test]
fn test_read_during_write() {
    let db = Arc::new(DatabaseCore::<MemoryStorage>::open_memory().unwrap());
    db.collection("rw_test").unwrap();

    let running = Arc::new(AtomicBool::new(true));
    let writes_done = Arc::new(AtomicU64::new(0));
    let reads_done = Arc::new(AtomicU64::new(0));

    // Writer thread
    let db_writer = Arc::clone(&db);
    let running_w = Arc::clone(&running);
    let writes = Arc::clone(&writes_done);
    let writer = thread::spawn(move || {
        let mut i = 0;
        while running_w.load(Ordering::Relaxed) {
            let doc = HashMap::from([("value".to_string(), json!(i))]);
            if db_writer.insert_one("rw_test", doc).is_ok() {
                writes.fetch_add(1, Ordering::Relaxed);
            }
            i += 1;
        }
    });

    // Reader threads
    let readers: Vec<_> = (0..5)
        .map(|_| {
            let db_reader = Arc::clone(&db);
            let running_r = Arc::clone(&running);
            let reads = Arc::clone(&reads_done);

            thread::spawn(move || {
                while running_r.load(Ordering::Relaxed) {
                    // These should never panic
                    let coll = db_reader.collection("rw_test").unwrap();
                    let _ = coll.find(&json!({}));
                    let _ = coll.count_documents(&json!({}));
                    reads.fetch_add(1, Ordering::Relaxed);
                }
            })
        })
        .collect();

    // Run for 1 second
    thread::sleep(Duration::from_secs(1));
    running.store(false, Ordering::Relaxed);

    writer.join().expect("Writer should not panic");
    for reader in readers {
        reader.join().expect("Reader should not panic");
    }

    println!(
        "Completed {} writes, {} reads",
        writes_done.load(Ordering::Relaxed),
        reads_done.load(Ordering::Relaxed)
    );

    // Final count should match
    let collection = db.collection("rw_test").unwrap();
    let final_count = collection.count_documents(&json!({})).unwrap();
    assert_eq!(final_count, writes_done.load(Ordering::Relaxed));
}

/// Test: Mixed CRUD operations concurrently
#[test]
fn test_mixed_crud_concurrent() {
    let db = Arc::new(DatabaseCore::<MemoryStorage>::open_memory().unwrap());
    db.collection("mixed").unwrap();

    // Pre-populate
    for i in 0..50 {
        let doc = HashMap::from([
            ("_id".to_string(), json!(i)),
            ("value".to_string(), json!(i)),
        ]);
        db.insert_one("mixed", doc).unwrap();
    }

    let running = Arc::new(AtomicBool::new(true));

    let handles: Vec<_> = (0..8)
        .map(|thread_id| {
            let db = Arc::clone(&db);
            let running = Arc::clone(&running);

            thread::spawn(move || {
                let mut rng_seed = thread_id as u64;
                while running.load(Ordering::Relaxed) {
                    // Simple LCG for determinism
                    rng_seed = rng_seed.wrapping_mul(6364136223846793005).wrapping_add(1);
                    let op = (rng_seed >> 32) % 4;

                    match op {
                        0 => {
                            // Insert
                            let doc = HashMap::from([("rnd".to_string(), json!(rng_seed))]);
                            let _ = db.insert_one("mixed", doc);
                        }
                        1 => {
                            // Find
                            let coll = db.collection("mixed").unwrap();
                            let _ = coll.find(&json!({}));
                        }
                        2 => {
                            // Update
                            let id = (rng_seed % 50) as i64;
                            let _ = db.update_one(
                                "mixed",
                                &json!({"_id": id}),
                                &json!({"$set": {"updated": true}}),
                            );
                        }
                        3 => {
                            // Count
                            let coll = db.collection("mixed").unwrap();
                            let _ = coll.count_documents(&json!({}));
                        }
                        _ => {}
                    }
                }
            })
        })
        .collect();

    thread::sleep(Duration::from_secs(1));
    running.store(false, Ordering::Relaxed);

    for handle in handles {
        handle.join().expect("Thread should not panic");
    }
}

// =============================================================================
// INDEX CONCURRENCY TESTS
// =============================================================================

/// Test: Concurrent index creation
#[test]
fn test_concurrent_index_creation() {
    let db = Arc::new(DatabaseCore::<MemoryStorage>::open_memory().unwrap());
    db.collection("idx_test").unwrap();

    // Pre-populate
    for i in 0..100 {
        let doc = HashMap::from([
            ("field_a".to_string(), json!(i)),
            ("field_b".to_string(), json!(i * 2)),
            ("field_c".to_string(), json!(format!("str_{}", i))),
        ]);
        db.insert_one("idx_test", doc).unwrap();
    }

    let barrier = Arc::new(Barrier::new(3));

    let fields = vec!["field_a", "field_b", "field_c"];
    let handles: Vec<_> = fields
        .into_iter()
        .map(|field| {
            let db = Arc::clone(&db);
            let barrier = Arc::clone(&barrier);
            let field_name = field.to_string();

            thread::spawn(move || {
                barrier.wait();
                let coll = db.collection("idx_test").unwrap();
                coll.create_index(field_name, false)
            })
        })
        .collect();

    for handle in handles {
        let result = handle.join().expect("Thread should not panic");
        assert!(result.is_ok(), "Index creation should succeed");
    }

    // Verify all indexes exist
    let collection = db.collection("idx_test").unwrap();
    let indexes = collection.list_indexes().unwrap();
    assert!(indexes.iter().any(|i| i.contains("field_a")));
    assert!(indexes.iter().any(|i| i.contains("field_b")));
    assert!(indexes.iter().any(|i| i.contains("field_c")));
}

/// Test: Insert during index operations
#[test]
fn test_insert_during_index_build() {
    let db = Arc::new(DatabaseCore::<MemoryStorage>::open_memory().unwrap());
    db.collection("idx_insert").unwrap();

    let barrier = Arc::new(Barrier::new(2));

    // Thread 1: Create index
    let db1 = Arc::clone(&db);
    let barrier1 = Arc::clone(&barrier);
    let indexer = thread::spawn(move || {
        barrier1.wait();
        let coll = db1.collection("idx_insert").unwrap();
        coll.create_index("value".to_string(), false)
    });

    // Thread 2: Insert documents
    let db2 = Arc::clone(&db);
    let barrier2 = Arc::clone(&barrier);
    let inserter = thread::spawn(move || {
        barrier2.wait();
        for i in 0..100 {
            let doc = HashMap::from([("value".to_string(), json!(i))]);
            db2.insert_one("idx_insert", doc).unwrap();
        }
    });

    indexer.join().unwrap().unwrap();
    inserter.join().unwrap();

    // Verify data
    let collection = db.collection("idx_insert").unwrap();
    let count = collection.count_documents(&json!({})).unwrap();
    assert_eq!(count, 100);
}

// =============================================================================
// LOCK CONTENTION TESTS
// =============================================================================

/// Test: Multiple collections accessed concurrently
#[test]
fn test_multiple_collections_concurrent() {
    let db = Arc::new(DatabaseCore::<MemoryStorage>::open_memory().unwrap());
    db.collection("coll_a").unwrap();
    db.collection("coll_b").unwrap();
    db.collection("coll_c").unwrap();

    let barrier = Arc::new(Barrier::new(3));

    // Thread 1: Work on coll_a
    let db1 = Arc::clone(&db);
    let b1 = Arc::clone(&barrier);
    let t1 = thread::spawn(move || {
        b1.wait();
        for i in 0..50 {
            db1.insert_one("coll_a", HashMap::from([("x".to_string(), json!(i))]))
                .unwrap();
        }
    });

    // Thread 2: Work on coll_b
    let db2 = Arc::clone(&db);
    let b2 = Arc::clone(&barrier);
    let t2 = thread::spawn(move || {
        b2.wait();
        for i in 0..50 {
            db2.insert_one("coll_b", HashMap::from([("y".to_string(), json!(i))]))
                .unwrap();
        }
    });

    // Thread 3: Work on coll_c
    let db3 = Arc::clone(&db);
    let b3 = Arc::clone(&barrier);
    let t3 = thread::spawn(move || {
        b3.wait();
        for i in 0..50 {
            db3.insert_one("coll_c", HashMap::from([("z".to_string(), json!(i))]))
                .unwrap();
        }
    });

    t1.join().unwrap();
    t2.join().unwrap();
    t3.join().unwrap();

    // Verify each collection
    let coll_a = db.collection("coll_a").unwrap();
    let coll_b = db.collection("coll_b").unwrap();
    let coll_c = db.collection("coll_c").unwrap();
    assert_eq!(coll_a.count_documents(&json!({})).unwrap(), 50);
    assert_eq!(coll_b.count_documents(&json!({})).unwrap(), 50);
    assert_eq!(coll_c.count_documents(&json!({})).unwrap(), 50);
}

/// Test: Detect potential deadlocks (with timeout)
#[test]
fn test_no_deadlock_with_cross_collection_access() {
    let db = Arc::new(DatabaseCore::<MemoryStorage>::open_memory().unwrap());
    db.collection("deadlock_a").unwrap();
    db.collection("deadlock_b").unwrap();

    let barrier = Arc::new(Barrier::new(2));

    // Thread 1: Access A then B
    let db1 = Arc::clone(&db);
    let b1 = Arc::clone(&barrier);
    let t1 = thread::spawn(move || {
        b1.wait();
        for i in 0..100 {
            db1.insert_one(
                "deadlock_a",
                HashMap::from([("from".to_string(), json!("t1"))]),
            )
            .unwrap();
            db1.insert_one(
                "deadlock_b",
                HashMap::from([("from".to_string(), json!("t1"))]),
            )
            .unwrap();
            if i % 10 == 0 {
                let coll_a = db1.collection("deadlock_a").unwrap();
                let _ = coll_a.find(&json!({}));
            }
        }
    });

    // Thread 2: Access B then A (opposite order)
    let db2 = Arc::clone(&db);
    let b2 = Arc::clone(&barrier);
    let t2 = thread::spawn(move || {
        b2.wait();
        for i in 0..100 {
            db2.insert_one(
                "deadlock_b",
                HashMap::from([("from".to_string(), json!("t2"))]),
            )
            .unwrap();
            db2.insert_one(
                "deadlock_a",
                HashMap::from([("from".to_string(), json!("t2"))]),
            )
            .unwrap();
            if i % 10 == 0 {
                let coll_b = db2.collection("deadlock_b").unwrap();
                let _ = coll_b.find(&json!({}));
            }
        }
    });

    // If this completes, no deadlock
    t1.join().expect("Thread 1 should complete");
    t2.join().expect("Thread 2 should complete");

    // Verify data
    let coll_a = db.collection("deadlock_a").unwrap();
    let coll_b = db.collection("deadlock_b").unwrap();
    let count_a = coll_a.count_documents(&json!({})).unwrap();
    let count_b = coll_b.count_documents(&json!({})).unwrap();
    assert_eq!(count_a, 200);
    assert_eq!(count_b, 200);
}

// =============================================================================
// QUERY CACHE CONCURRENCY TESTS
// =============================================================================

/// Test: Cache invalidation during concurrent reads
#[test]
fn test_cache_invalidation_during_reads() {
    let db = Arc::new(DatabaseCore::<MemoryStorage>::open_memory().unwrap());
    db.collection("cache_test").unwrap();

    // Pre-populate
    for i in 0..50 {
        db.insert_one(
            "cache_test",
            HashMap::from([("value".to_string(), json!(i))]),
        )
        .unwrap();
    }

    let running = Arc::new(AtomicBool::new(true));

    // Reader threads - use same query (should hit cache)
    let readers: Vec<_> = (0..5)
        .map(|_| {
            let db = Arc::clone(&db);
            let running = Arc::clone(&running);

            thread::spawn(move || {
                while running.load(Ordering::Relaxed) {
                    let coll = db.collection("cache_test").unwrap();
                    let _ = coll.find(&json!({"value": {"$gte": 25}}));
                }
            })
        })
        .collect();

    // Writer thread - invalidates cache
    let db_writer = Arc::clone(&db);
    let running_w = Arc::clone(&running);
    let writer = thread::spawn(move || {
        let mut i = 100;
        while running_w.load(Ordering::Relaxed) {
            let doc = HashMap::from([("value".to_string(), json!(i))]);
            let _ = db_writer.insert_one("cache_test", doc);
            i += 1;
        }
    });

    thread::sleep(Duration::from_millis(500));
    running.store(false, Ordering::Relaxed);

    writer.join().unwrap();
    for r in readers {
        r.join().unwrap();
    }
}

// =============================================================================
// STRESS TESTS
// =============================================================================

/// Test: High contention scenario
#[test]
fn test_high_contention_stress() {
    let db = Arc::new(DatabaseCore::<MemoryStorage>::open_memory().unwrap());
    db.collection("high_contention").unwrap();

    const NUM_THREADS: usize = 20;
    const OPS_PER_THREAD: usize = 50;

    let barrier = Arc::new(Barrier::new(NUM_THREADS));
    let total_ops = Arc::new(AtomicU64::new(0));

    let handles: Vec<_> = (0..NUM_THREADS)
        .map(|thread_id| {
            let db = Arc::clone(&db);
            let barrier = Arc::clone(&barrier);
            let ops = Arc::clone(&total_ops);

            thread::spawn(move || {
                barrier.wait();

                for i in 0..OPS_PER_THREAD {
                    // Alternate between operations
                    match i % 3 {
                        0 => {
                            let doc = HashMap::from([
                                ("t".to_string(), json!(thread_id)),
                                ("i".to_string(), json!(i)),
                            ]);
                            db.insert_one("high_contention", doc).unwrap();
                        }
                        1 => {
                            let coll = db.collection("high_contention").unwrap();
                            let _ = coll.find(&json!({}));
                        }
                        2 => {
                            let coll = db.collection("high_contention").unwrap();
                            let _ = coll.count_documents(&json!({}));
                        }
                        _ => {}
                    }
                    ops.fetch_add(1, Ordering::Relaxed);
                }
            })
        })
        .collect();

    for handle in handles {
        handle.join().expect("Thread should not panic");
    }

    println!("Completed {} operations", total_ops.load(Ordering::Relaxed));

    // Should have inserted ~1/3 of operations
    let expected_inserts = (NUM_THREADS * OPS_PER_THREAD) / 3;
    let collection = db.collection("high_contention").unwrap();
    let count = collection.count_documents(&json!({})).unwrap();
    assert!(
        count >= expected_inserts as u64 - 50,
        "Should have ~{} inserts, got {}",
        expected_inserts,
        count
    );
}

/// Test: Long-running concurrent operations
#[test]
fn test_sustained_concurrent_load() {
    let db = Arc::new(DatabaseCore::<MemoryStorage>::open_memory().unwrap());
    db.collection("sustained").unwrap();

    let running = Arc::new(AtomicBool::new(true));
    let ops_count = Arc::new(AtomicU64::new(0));

    let handles: Vec<_> = (0..10)
        .map(|_| {
            let db = Arc::clone(&db);
            let running = Arc::clone(&running);
            let ops = Arc::clone(&ops_count);

            thread::spawn(move || {
                while running.load(Ordering::Relaxed) {
                    let doc = HashMap::from([(
                        "timestamp".to_string(),
                        json!(ops.load(Ordering::Relaxed)),
                    )]);
                    let _ = db.insert_one("sustained", doc);
                    let coll = db.collection("sustained").unwrap();
                    let _ = coll.find(&json!({}));
                    ops.fetch_add(2, Ordering::Relaxed);
                }
            })
        })
        .collect();

    // Run for 2 seconds
    thread::sleep(Duration::from_secs(2));
    running.store(false, Ordering::Relaxed);

    for handle in handles {
        handle.join().unwrap();
    }

    let total_ops = ops_count.load(Ordering::Relaxed);
    println!("Sustained load: {} operations in 2 seconds", total_ops);
    assert!(total_ops > 100, "Should complete many operations");
}

// =============================================================================
// EDGE CASE TESTS
// =============================================================================

/// Test: Empty collection concurrent access
#[test]
fn test_empty_collection_concurrent() {
    let db = Arc::new(DatabaseCore::<MemoryStorage>::open_memory().unwrap());
    db.collection("empty").unwrap();

    // Multiple threads querying empty collection
    let handles: Vec<_> = (0..10)
        .map(|_| {
            let db = Arc::clone(&db);
            thread::spawn(move || {
                for _ in 0..100 {
                    let coll = db.collection("empty").unwrap();
                    let results = coll.find(&json!({})).unwrap();
                    assert_eq!(results.len(), 0);
                    let count = coll.count_documents(&json!({})).unwrap();
                    assert_eq!(count, 0);
                }
            })
        })
        .collect();

    for handle in handles {
        handle.join().unwrap();
    }
}

/// Test: Single document high contention
#[test]
fn test_single_document_high_contention() {
    let db = Arc::new(DatabaseCore::<MemoryStorage>::open_memory().unwrap());
    db.collection("single_doc").unwrap();

    // Insert one document
    db.insert_one(
        "single_doc",
        HashMap::from([
            ("_id".to_string(), json!(1)),
            ("counter".to_string(), json!(0)),
        ]),
    )
    .unwrap();

    let update_count = Arc::new(AtomicU64::new(0));

    // Multiple threads updating same document
    let handles: Vec<_> = (0..10)
        .map(|_| {
            let db = Arc::clone(&db);
            let updates = Arc::clone(&update_count);

            thread::spawn(move || {
                for _ in 0..50 {
                    let result = db.update_one(
                        "single_doc",
                        &json!({"_id": 1}),
                        &json!({"$inc": {"counter": 1}}),
                    );
                    if result.is_ok() {
                        updates.fetch_add(1, Ordering::Relaxed);
                    }
                }
            })
        })
        .collect();

    for handle in handles {
        handle.join().unwrap();
    }

    println!(
        "Completed {} updates on single document",
        update_count.load(Ordering::Relaxed)
    );
}

// =============================================================================
// BUG #4: UNIQUE CONSTRAINT RACE CONDITION TEST
// =============================================================================
// This test verifies that concurrent inserts with the same unique key value
// properly enforce the unique constraint. Only ONE thread should succeed,
// and the database should remain consistent.
//
// The race condition occurs when:
// 1. Thread A: prepare (constraint check) -> OK
// 2. Thread B: prepare (constraint check) -> OK (race window!)
// 3. Thread A: WAL commit -> COMMITTED
// 4. Thread B: WAL commit -> COMMITTED
// 5. Thread B: persist (index write) -> OK
// 6. Thread A: persist (index write) -> FAIL (duplicate key)
//
// Result: A gets error but WAL is committed, B succeeds.
// After recovery: both documents in storage, but only one in index!

/// Test: Concurrent inserts with unique constraint - MUST enforce atomicity
///
/// Expected behavior (CORRECT):
/// - Exactly 1 insert succeeds
/// - Exactly N-1 inserts fail with duplicate key error
/// - After all threads complete: exactly 1 document in storage
/// - Index query returns exactly 1 document
///
/// Bug manifestation (INCORRECT):
/// - Multiple inserts succeed (race through constraint check)
/// - After recovery: documents in storage but missing from index
///
/// NOTE: This test uses StorageEngine (file-based) because the race condition
/// only affects Safe mode with WAL (prepare/persist separation).
/// MemoryStorage uses insert_one_raw which is atomic.
#[test]
fn test_concurrent_unique_constraint_race_bug4() {
    const NUM_THREADS: usize = 10;
    const EMAIL_VALUE: &str = "race@test.com";

    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("race_test.mlite");

    let db = Arc::new(DatabaseCore::open(&db_path).unwrap());
    let collection = db.collection("users").unwrap();

    // Create unique index on email field
    collection
        .create_index("email".to_string(), true)
        .expect("Index creation should succeed");

    let barrier = Arc::new(Barrier::new(NUM_THREADS));

    // All threads try to insert the SAME email value simultaneously
    let handles: Vec<_> = (0..NUM_THREADS)
        .map(|thread_id| {
            let db = Arc::clone(&db);
            let barrier = Arc::clone(&barrier);

            thread::spawn(move || {
                barrier.wait(); // All threads start at exactly the same time

                let doc = HashMap::from([
                    ("email".to_string(), json!(EMAIL_VALUE)),
                    ("thread_id".to_string(), json!(thread_id)),
                ]);

                db.insert_one("users", doc)
            })
        })
        .collect();

    // Collect results
    let results: Vec<_> = handles
        .into_iter()
        .map(|h| h.join().expect("Thread should not panic"))
        .collect();

    let successes: Vec<_> = results.iter().filter(|r| r.is_ok()).collect();
    let failures: Vec<_> = results.iter().filter(|r| r.is_err()).collect();

    println!(
        "Results: {} successes, {} failures",
        successes.len(),
        failures.len()
    );

    // CRITICAL ASSERTION #1: Exactly ONE insert should succeed
    assert_eq!(
        successes.len(),
        1,
        "BUG #4: Race condition detected! Expected exactly 1 success, got {}. \
         Multiple threads passed the unique constraint check simultaneously.",
        successes.len()
    );

    // CRITICAL ASSERTION #2: All other inserts should fail
    assert_eq!(
        failures.len(),
        NUM_THREADS - 1,
        "Expected {} failures, got {}",
        NUM_THREADS - 1,
        failures.len()
    );

    // CRITICAL ASSERTION #3: Storage should have exactly 1 document
    let collection = db.collection("users").unwrap();
    let all_docs = collection.find(&json!({})).unwrap();
    assert_eq!(
        all_docs.len(),
        1,
        "BUG #4: Storage inconsistency! Expected 1 document, found {}",
        all_docs.len()
    );

    // CRITICAL ASSERTION #4: Index query should return exactly 1 document
    let via_index = collection.find(&json!({"email": EMAIL_VALUE})).unwrap();
    assert_eq!(
        via_index.len(),
        1,
        "BUG #4: Index inconsistency! Query via unique index returned {} documents",
        via_index.len()
    );

    // CRITICAL ASSERTION #5: count_documents should match
    let count = collection.count_documents(&json!({})).unwrap();
    assert_eq!(count, 1, "count_documents mismatch");

    println!("BUG #4 test passed: Unique constraint properly enforced under concurrency");
}

/// Test: Multiple different unique values - should all succeed
#[test]
fn test_concurrent_unique_different_values() {
    const NUM_THREADS: usize = 10;

    let db = Arc::new(DatabaseCore::<MemoryStorage>::open_memory().unwrap());
    let collection = db.collection("users").unwrap();

    // Create unique index on email field
    collection
        .create_index("email".to_string(), true)
        .expect("Index creation should succeed");

    let barrier = Arc::new(Barrier::new(NUM_THREADS));

    // Each thread inserts a DIFFERENT email value
    let handles: Vec<_> = (0..NUM_THREADS)
        .map(|thread_id| {
            let db = Arc::clone(&db);
            let barrier = Arc::clone(&barrier);

            thread::spawn(move || {
                barrier.wait();

                let doc = HashMap::from([
                    (
                        "email".to_string(),
                        json!(format!("user{}@test.com", thread_id)),
                    ),
                    ("thread_id".to_string(), json!(thread_id)),
                ]);

                db.insert_one("users", doc)
            })
        })
        .collect();

    let results: Vec<_> = handles
        .into_iter()
        .map(|h| h.join().expect("Thread should not panic"))
        .collect();

    let successes = results.iter().filter(|r| r.is_ok()).count();
    assert_eq!(
        successes, NUM_THREADS,
        "All inserts with different unique values should succeed"
    );

    let collection = db.collection("users").unwrap();
    let count = collection.count_documents(&json!({})).unwrap();
    assert_eq!(count, NUM_THREADS as u64);
}
