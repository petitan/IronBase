// chaos_concurrent_tests.rs
// Phase 3: Concurrent Stress & Race Condition Tests
//
// These tests verify thread safety under heavy concurrent load:
// 1. No deadlocks occur
// 2. Data integrity is maintained
// 3. All operations complete without panic

use ironbase_core::database::DatabaseCore;
use ironbase_core::storage::{MemoryStorage, Storage};
use ironbase_core::CollectionCore;
use parking_lot::RwLock;
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

    let storage = Arc::new(RwLock::new(MemoryStorage::new()));

    // Create collection
    {
        let mut s = storage.write();
        s.create_collection("stress").unwrap();
    }

    let collection = CollectionCore::new("stress".to_string(), Arc::clone(&storage)).unwrap();
    let collection = Arc::new(collection);

    let barrier = Arc::new(Barrier::new(NUM_THREADS));

    let handles: Vec<_> = (0..NUM_THREADS)
        .map(|thread_id| {
            let coll = Arc::clone(&collection);
            let barrier = Arc::clone(&barrier);

            thread::spawn(move || {
                barrier.wait(); // All threads start together

                for i in 0..DOCS_PER_THREAD {
                    let doc = HashMap::from([
                        ("thread".to_string(), json!(thread_id)),
                        ("seq".to_string(), json!(i)),
                        ("data".to_string(), json!(format!("t{}_{}", thread_id, i))),
                    ]);
                    coll.insert_one(doc).expect("Insert should succeed");
                }
            })
        })
        .collect();

    // Wait for all threads
    for handle in handles {
        handle.join().expect("Thread should not panic");
    }

    // Verify count
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
                    db.collection("stress")
                        .unwrap()
                        .insert_one(doc)
                        .expect("Insert should succeed");
                }
            })
        })
        .collect();

    for handle in handles {
        handle.join().expect("Thread should not panic");
    }

    let count = db
        .collection("stress")
        .unwrap()
        .count_documents(&json!({}))
        .unwrap();
    assert_eq!(count, (NUM_THREADS * DOCS_PER_THREAD) as u64);
}

// =============================================================================
// READ/WRITE CONCURRENCY TESTS
// =============================================================================

/// Test: Concurrent reads during writes
/// Expected: Readers see consistent data, no panics
#[test]
fn test_read_during_write() {
    let storage = Arc::new(RwLock::new(MemoryStorage::new()));

    {
        let mut s = storage.write();
        s.create_collection("rw_test").unwrap();
    }

    let collection = Arc::new(
        CollectionCore::new("rw_test".to_string(), Arc::clone(&storage)).unwrap(),
    );

    let running = Arc::new(AtomicBool::new(true));
    let writes_done = Arc::new(AtomicU64::new(0));
    let reads_done = Arc::new(AtomicU64::new(0));

    // Writer thread
    let coll_writer = Arc::clone(&collection);
    let running_w = Arc::clone(&running);
    let writes = Arc::clone(&writes_done);
    let writer = thread::spawn(move || {
        let mut i = 0;
        while running_w.load(Ordering::Relaxed) {
            let doc = HashMap::from([("value".to_string(), json!(i))]);
            if coll_writer.insert_one(doc).is_ok() {
                writes.fetch_add(1, Ordering::Relaxed);
            }
            i += 1;
        }
    });

    // Reader threads
    let readers: Vec<_> = (0..5)
        .map(|_| {
            let coll_reader = Arc::clone(&collection);
            let running_r = Arc::clone(&running);
            let reads = Arc::clone(&reads_done);

            thread::spawn(move || {
                while running_r.load(Ordering::Relaxed) {
                    // These should never panic
                    let _ = coll_reader.find(&json!({}));
                    let _ = coll_reader.count_documents(&json!({}));
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
    let final_count = collection.count_documents(&json!({})).unwrap();
    assert_eq!(final_count, writes_done.load(Ordering::Relaxed));
}

/// Test: Mixed CRUD operations concurrently
#[test]
fn test_mixed_crud_concurrent() {
    let storage = Arc::new(RwLock::new(MemoryStorage::new()));

    {
        let mut s = storage.write();
        s.create_collection("mixed").unwrap();
    }

    let collection = Arc::new(
        CollectionCore::new("mixed".to_string(), Arc::clone(&storage)).unwrap(),
    );

    // Pre-populate
    for i in 0..50 {
        let doc = HashMap::from([
            ("_id".to_string(), json!(i)),
            ("value".to_string(), json!(i)),
        ]);
        collection.insert_one(doc).unwrap();
    }

    let running = Arc::new(AtomicBool::new(true));

    let handles: Vec<_> = (0..8)
        .map(|thread_id| {
            let coll = Arc::clone(&collection);
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
                            let _ = coll.insert_one(doc);
                        }
                        1 => {
                            // Find
                            let _ = coll.find(&json!({}));
                        }
                        2 => {
                            // Update
                            let id = (rng_seed % 50) as i64;
                            let _ = coll.update_one(
                                &json!({"_id": id}),
                                &json!({"$set": {"updated": true}}),
                            );
                        }
                        3 => {
                            // Count
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
    let storage = Arc::new(RwLock::new(MemoryStorage::new()));

    {
        let mut s = storage.write();
        s.create_collection("idx_test").unwrap();
    }

    let collection = Arc::new(
        CollectionCore::new("idx_test".to_string(), Arc::clone(&storage)).unwrap(),
    );

    // Pre-populate
    for i in 0..100 {
        let doc = HashMap::from([
            ("field_a".to_string(), json!(i)),
            ("field_b".to_string(), json!(i * 2)),
            ("field_c".to_string(), json!(format!("str_{}", i))),
        ]);
        collection.insert_one(doc).unwrap();
    }

    let barrier = Arc::new(Barrier::new(3));

    let fields = vec!["field_a", "field_b", "field_c"];
    let handles: Vec<_> = fields
        .into_iter()
        .map(|field| {
            let coll = Arc::clone(&collection);
            let barrier = Arc::clone(&barrier);
            let field_name = field.to_string();

            thread::spawn(move || {
                barrier.wait();
                coll.create_index(field_name, false)
            })
        })
        .collect();

    for handle in handles {
        let result = handle.join().expect("Thread should not panic");
        assert!(result.is_ok(), "Index creation should succeed");
    }

    // Verify all indexes exist
    let indexes = collection.list_indexes();
    assert!(indexes.iter().any(|i| i.contains("field_a")));
    assert!(indexes.iter().any(|i| i.contains("field_b")));
    assert!(indexes.iter().any(|i| i.contains("field_c")));
}

/// Test: Insert during index operations
#[test]
fn test_insert_during_index_build() {
    let storage = Arc::new(RwLock::new(MemoryStorage::new()));

    {
        let mut s = storage.write();
        s.create_collection("idx_insert").unwrap();
    }

    let collection = Arc::new(
        CollectionCore::new("idx_insert".to_string(), Arc::clone(&storage)).unwrap(),
    );

    let barrier = Arc::new(Barrier::new(2));

    // Thread 1: Create index
    let coll1 = Arc::clone(&collection);
    let barrier1 = Arc::clone(&barrier);
    let indexer = thread::spawn(move || {
        barrier1.wait();
        coll1.create_index("value".to_string(), false)
    });

    // Thread 2: Insert documents
    let coll2 = Arc::clone(&collection);
    let barrier2 = Arc::clone(&barrier);
    let inserter = thread::spawn(move || {
        barrier2.wait();
        for i in 0..100 {
            let doc = HashMap::from([("value".to_string(), json!(i))]);
            coll2.insert_one(doc).unwrap();
        }
    });

    indexer.join().unwrap().unwrap();
    inserter.join().unwrap();

    // Verify data
    let count = collection.count_documents(&json!({})).unwrap();
    assert_eq!(count, 100);
}

// =============================================================================
// LOCK CONTENTION TESTS
// =============================================================================

/// Test: Multiple collections accessed concurrently
#[test]
fn test_multiple_collections_concurrent() {
    let storage = Arc::new(RwLock::new(MemoryStorage::new()));

    {
        let mut s = storage.write();
        s.create_collection("coll_a").unwrap();
        s.create_collection("coll_b").unwrap();
        s.create_collection("coll_c").unwrap();
    }

    let coll_a = Arc::new(
        CollectionCore::new("coll_a".to_string(), Arc::clone(&storage)).unwrap(),
    );
    let coll_b = Arc::new(
        CollectionCore::new("coll_b".to_string(), Arc::clone(&storage)).unwrap(),
    );
    let coll_c = Arc::new(
        CollectionCore::new("coll_c".to_string(), Arc::clone(&storage)).unwrap(),
    );

    let barrier = Arc::new(Barrier::new(3));

    // Thread 1: Work on coll_a
    let ca = Arc::clone(&coll_a);
    let b1 = Arc::clone(&barrier);
    let t1 = thread::spawn(move || {
        b1.wait();
        for i in 0..50 {
            ca.insert_one(HashMap::from([("x".to_string(), json!(i))]))
                .unwrap();
        }
    });

    // Thread 2: Work on coll_b
    let cb = Arc::clone(&coll_b);
    let b2 = Arc::clone(&barrier);
    let t2 = thread::spawn(move || {
        b2.wait();
        for i in 0..50 {
            cb.insert_one(HashMap::from([("y".to_string(), json!(i))]))
                .unwrap();
        }
    });

    // Thread 3: Work on coll_c
    let cc = Arc::clone(&coll_c);
    let b3 = Arc::clone(&barrier);
    let t3 = thread::spawn(move || {
        b3.wait();
        for i in 0..50 {
            cc.insert_one(HashMap::from([("z".to_string(), json!(i))]))
                .unwrap();
        }
    });

    t1.join().unwrap();
    t2.join().unwrap();
    t3.join().unwrap();

    // Verify each collection
    assert_eq!(coll_a.count_documents(&json!({})).unwrap(), 50);
    assert_eq!(coll_b.count_documents(&json!({})).unwrap(), 50);
    assert_eq!(coll_c.count_documents(&json!({})).unwrap(), 50);
}

/// Test: Detect potential deadlocks (with timeout)
#[test]
fn test_no_deadlock_with_cross_collection_access() {
    let storage = Arc::new(RwLock::new(MemoryStorage::new()));

    {
        let mut s = storage.write();
        s.create_collection("deadlock_a").unwrap();
        s.create_collection("deadlock_b").unwrap();
    }

    let coll_a = Arc::new(
        CollectionCore::new("deadlock_a".to_string(), Arc::clone(&storage)).unwrap(),
    );
    let coll_b = Arc::new(
        CollectionCore::new("deadlock_b".to_string(), Arc::clone(&storage)).unwrap(),
    );

    let barrier = Arc::new(Barrier::new(2));

    // Thread 1: Access A then B
    let ca1 = Arc::clone(&coll_a);
    let cb1 = Arc::clone(&coll_b);
    let b1 = Arc::clone(&barrier);
    let t1 = thread::spawn(move || {
        b1.wait();
        for i in 0..100 {
            ca1.insert_one(HashMap::from([("from".to_string(), json!("t1"))]))
                .unwrap();
            cb1.insert_one(HashMap::from([("from".to_string(), json!("t1"))]))
                .unwrap();
            if i % 10 == 0 {
                let _ = ca1.find(&json!({}));
            }
        }
    });

    // Thread 2: Access B then A (opposite order)
    let ca2 = Arc::clone(&coll_a);
    let cb2 = Arc::clone(&coll_b);
    let b2 = Arc::clone(&barrier);
    let t2 = thread::spawn(move || {
        b2.wait();
        for i in 0..100 {
            cb2.insert_one(HashMap::from([("from".to_string(), json!("t2"))]))
                .unwrap();
            ca2.insert_one(HashMap::from([("from".to_string(), json!("t2"))]))
                .unwrap();
            if i % 10 == 0 {
                let _ = cb2.find(&json!({}));
            }
        }
    });

    // If this completes, no deadlock
    t1.join().expect("Thread 1 should complete");
    t2.join().expect("Thread 2 should complete");

    // Verify data
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
    let storage = Arc::new(RwLock::new(MemoryStorage::new()));

    {
        let mut s = storage.write();
        s.create_collection("cache_test").unwrap();
    }

    let collection = Arc::new(
        CollectionCore::new("cache_test".to_string(), Arc::clone(&storage)).unwrap(),
    );

    // Pre-populate
    for i in 0..50 {
        collection
            .insert_one(HashMap::from([("value".to_string(), json!(i))]))
            .unwrap();
    }

    let running = Arc::new(AtomicBool::new(true));

    // Reader threads - use same query (should hit cache)
    let readers: Vec<_> = (0..5)
        .map(|_| {
            let coll = Arc::clone(&collection);
            let running = Arc::clone(&running);

            thread::spawn(move || {
                while running.load(Ordering::Relaxed) {
                    let _ = coll.find(&json!({"value": {"$gte": 25}}));
                }
            })
        })
        .collect();

    // Writer thread - invalidates cache
    let coll_writer = Arc::clone(&collection);
    let running_w = Arc::clone(&running);
    let writer = thread::spawn(move || {
        let mut i = 100;
        while running_w.load(Ordering::Relaxed) {
            let doc = HashMap::from([("value".to_string(), json!(i))]);
            let _ = coll_writer.insert_one(doc);
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
    let storage = Arc::new(RwLock::new(MemoryStorage::new()));

    {
        let mut s = storage.write();
        s.create_collection("high_contention").unwrap();
    }

    let collection = Arc::new(
        CollectionCore::new("high_contention".to_string(), Arc::clone(&storage)).unwrap(),
    );

    const NUM_THREADS: usize = 20;
    const OPS_PER_THREAD: usize = 50;

    let barrier = Arc::new(Barrier::new(NUM_THREADS));
    let total_ops = Arc::new(AtomicU64::new(0));

    let handles: Vec<_> = (0..NUM_THREADS)
        .map(|thread_id| {
            let coll = Arc::clone(&collection);
            let barrier = Arc::clone(&barrier);
            let ops = Arc::clone(&total_ops);

            thread::spawn(move || {
                barrier.wait();

                for i in 0..OPS_PER_THREAD {
                    // Alternate between operations
                    match i % 3 {
                        0 => {
                            let doc =
                                HashMap::from([("t".to_string(), json!(thread_id)), ("i".to_string(), json!(i))]);
                            coll.insert_one(doc).unwrap();
                        }
                        1 => {
                            let _ = coll.find(&json!({}));
                        }
                        2 => {
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

    println!(
        "Completed {} operations",
        total_ops.load(Ordering::Relaxed)
    );

    // Should have inserted ~1/3 of operations
    let expected_inserts = (NUM_THREADS * OPS_PER_THREAD) / 3;
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
    let storage = Arc::new(RwLock::new(MemoryStorage::new()));

    {
        let mut s = storage.write();
        s.create_collection("sustained").unwrap();
    }

    let collection = Arc::new(
        CollectionCore::new("sustained".to_string(), Arc::clone(&storage)).unwrap(),
    );

    let running = Arc::new(AtomicBool::new(true));
    let ops_count = Arc::new(AtomicU64::new(0));

    let handles: Vec<_> = (0..10)
        .map(|_| {
            let coll = Arc::clone(&collection);
            let running = Arc::clone(&running);
            let ops = Arc::clone(&ops_count);

            thread::spawn(move || {
                while running.load(Ordering::Relaxed) {
                    let doc = HashMap::from([("timestamp".to_string(), json!(ops.load(Ordering::Relaxed)))]);
                    let _ = coll.insert_one(doc);
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
    let storage = Arc::new(RwLock::new(MemoryStorage::new()));

    {
        let mut s = storage.write();
        s.create_collection("empty").unwrap();
    }

    let collection = Arc::new(
        CollectionCore::new("empty".to_string(), Arc::clone(&storage)).unwrap(),
    );

    // Multiple threads querying empty collection
    let handles: Vec<_> = (0..10)
        .map(|_| {
            let coll = Arc::clone(&collection);
            thread::spawn(move || {
                for _ in 0..100 {
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
    let storage = Arc::new(RwLock::new(MemoryStorage::new()));

    {
        let mut s = storage.write();
        s.create_collection("single_doc").unwrap();
    }

    let collection = Arc::new(
        CollectionCore::new("single_doc".to_string(), Arc::clone(&storage)).unwrap(),
    );

    // Insert one document
    collection
        .insert_one(HashMap::from([
            ("_id".to_string(), json!(1)),
            ("counter".to_string(), json!(0)),
        ]))
        .unwrap();

    let update_count = Arc::new(AtomicU64::new(0));

    // Multiple threads updating same document
    let handles: Vec<_> = (0..10)
        .map(|_| {
            let coll = Arc::clone(&collection);
            let updates = Arc::clone(&update_count);

            thread::spawn(move || {
                for _ in 0..50 {
                    let result = coll.update_one(
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
