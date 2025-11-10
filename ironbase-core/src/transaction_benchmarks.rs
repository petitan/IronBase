// ironbase-core/src/transaction_benchmarks.rs
// Manual benchmarks for ACD transactions (without criterion)

#[cfg(test)]
mod benchmarks {
    use crate::database::DatabaseCore;
    use crate::transaction::Operation;
    use crate::document::DocumentId;
    use serde_json::json;
    use tempfile::TempDir;
    use std::time::Instant;

    /// Helper to format duration nicely
    fn format_duration(nanos: u128) -> String {
        if nanos < 1_000 {
            format!("{} ns", nanos)
        } else if nanos < 1_000_000 {
            format!("{:.2} µs", nanos as f64 / 1_000.0)
        } else if nanos < 1_000_000_000 {
            format!("{:.2} ms", nanos as f64 / 1_000_000.0)
        } else {
            format!("{:.2} s", nanos as f64 / 1_000_000_000.0)
        }
    }

    #[test]
    fn bench_empty_transaction_overhead() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("bench.mlite");
        let db = DatabaseCore::open(&db_path).unwrap();

        let iterations = 1000;
        let start = Instant::now();

        for _ in 0..iterations {
            let tx_id = db.begin_transaction();
            db.commit_transaction(tx_id).unwrap();
        }

        let elapsed = start.elapsed();
        let avg_nanos = elapsed.as_nanos() / iterations;

        println!("\n📊 Empty Transaction Overhead:");
        println!("   Total: {} iterations in {:?}", iterations, elapsed);
        println!("   Average: {} per transaction", format_duration(avg_nanos));
        println!("   Throughput: {:.0} tx/sec", iterations as f64 / elapsed.as_secs_f64());
    }

    #[test]
    fn bench_single_operation_transaction() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("bench.mlite");
        let db = DatabaseCore::open(&db_path).unwrap();

        db.collection("bench").unwrap();

        let iterations = 1000;
        let start = Instant::now();

        for i in 0..iterations {
            let tx_id = db.begin_transaction();
            let mut tx = db.get_transaction(tx_id).unwrap();

            tx.add_operation(Operation::Insert {
                collection: "bench".to_string(),
                doc_id: DocumentId::Int(i as i64),
                doc: json!({"id": i, "data": "test"}),
            }).unwrap();

            db.update_transaction(tx_id, tx).unwrap();
            db.commit_transaction(tx_id).unwrap();
        }

        let elapsed = start.elapsed();
        let avg_nanos = elapsed.as_nanos() / iterations;

        println!("\n📊 Single Operation Transaction:");
        println!("   Total: {} iterations in {:?}", iterations, elapsed);
        println!("   Average: {} per transaction", format_duration(avg_nanos));
        println!("   Throughput: {:.0} tx/sec", iterations as f64 / elapsed.as_secs_f64());
    }

    #[test]
    fn bench_10_operation_transaction() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("bench.mlite");
        let db = DatabaseCore::open(&db_path).unwrap();

        db.collection("bench").unwrap();

        let iterations = 100;
        let ops_per_tx = 10;
        let start = Instant::now();

        for batch in 0..iterations {
            let tx_id = db.begin_transaction();
            let mut tx = db.get_transaction(tx_id).unwrap();

            for i in 0..ops_per_tx {
                let doc_id = batch * ops_per_tx + i;
                tx.add_operation(Operation::Insert {
                    collection: "bench".to_string(),
                    doc_id: DocumentId::Int(doc_id as i64),
                    doc: json!({"id": doc_id, "data": format!("item_{}", doc_id)}),
                }).unwrap();
            }

            db.update_transaction(tx_id, tx).unwrap();
            db.commit_transaction(tx_id).unwrap();
        }

        let elapsed = start.elapsed();
        let avg_nanos = elapsed.as_nanos() / iterations;

        println!("\n📊 10-Operation Transaction:");
        println!("   Total: {} transactions in {:?}", iterations, elapsed);
        println!("   Average: {} per transaction", format_duration(avg_nanos));
        println!("   Throughput: {:.0} tx/sec", iterations as f64 / elapsed.as_secs_f64());
        println!("   Per operation: {}", format_duration(avg_nanos / ops_per_tx));
    }

    #[test]
    fn bench_100_operation_transaction() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("bench.mlite");
        let db = DatabaseCore::open(&db_path).unwrap();

        db.collection("bench").unwrap();

        let iterations = 10;
        let ops_per_tx = 100;
        let start = Instant::now();

        for batch in 0..iterations {
            let tx_id = db.begin_transaction();
            let mut tx = db.get_transaction(tx_id).unwrap();

            for i in 0..ops_per_tx {
                let doc_id = batch * ops_per_tx + i;
                tx.add_operation(Operation::Insert {
                    collection: "bench".to_string(),
                    doc_id: DocumentId::Int(doc_id as i64),
                    doc: json!({"id": doc_id, "value": doc_id * 2}),
                }).unwrap();
            }

            db.update_transaction(tx_id, tx).unwrap();
            db.commit_transaction(tx_id).unwrap();
        }

        let elapsed = start.elapsed();
        let avg_nanos = elapsed.as_nanos() / iterations;

        println!("\n📊 100-Operation Transaction:");
        println!("   Total: {} transactions in {:?}", iterations, elapsed);
        println!("   Average: {} per transaction", format_duration(avg_nanos));
        println!("   Throughput: {:.0} tx/sec", iterations as f64 / elapsed.as_secs_f64());
        println!("   Per operation: {}", format_duration(avg_nanos / ops_per_tx));
    }

    #[test]
    fn bench_rollback_overhead() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("bench.mlite");
        let db = DatabaseCore::open(&db_path).unwrap();

        db.collection("bench").unwrap();

        let iterations = 1000;
        let start = Instant::now();

        for i in 0..iterations {
            let tx_id = db.begin_transaction();
            let mut tx = db.get_transaction(tx_id).unwrap();

            // Add some operations
            for j in 0..5 {
                tx.add_operation(Operation::Insert {
                    collection: "bench".to_string(),
                    doc_id: DocumentId::Int((i * 5 + j) as i64),
                    doc: json!({"data": "test"}),
                }).unwrap();
            }

            db.update_transaction(tx_id, tx).unwrap();

            // Rollback instead of commit
            db.rollback_transaction(tx_id).unwrap();
        }

        let elapsed = start.elapsed();
        let avg_nanos = elapsed.as_nanos() / iterations;

        println!("\n📊 Rollback Overhead (5 ops each):");
        println!("   Total: {} iterations in {:?}", iterations, elapsed);
        println!("   Average: {} per rollback", format_duration(avg_nanos));
        println!("   Throughput: {:.0} rollbacks/sec", iterations as f64 / elapsed.as_secs_f64());
    }

    #[test]
    fn bench_begin_transaction_only() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("bench.mlite");
        let db = DatabaseCore::open(&db_path).unwrap();

        let iterations = 10000;
        let start = Instant::now();

        for _ in 0..iterations {
            let _tx_id = db.begin_transaction();
        }

        let elapsed = start.elapsed();
        let avg_nanos = elapsed.as_nanos() / iterations;

        println!("\n📊 Begin Transaction Only:");
        println!("   Total: {} iterations in {:?}", iterations, elapsed);
        println!("   Average: {} per begin", format_duration(avg_nanos));
        println!("   Throughput: {:.0} begins/sec", iterations as f64 / elapsed.as_secs_f64());
    }

    #[test]
    fn bench_wal_write_performance() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("bench.mlite");
        let db = DatabaseCore::open(&db_path).unwrap();

        db.collection("wal_bench").unwrap();

        let iterations = 100;
        let start = Instant::now();

        for i in 0..iterations {
            let tx_id = db.begin_transaction();
            let mut tx = db.get_transaction(tx_id).unwrap();

            // Single insert to measure WAL write overhead
            tx.add_operation(Operation::Insert {
                collection: "wal_bench".to_string(),
                doc_id: DocumentId::Int(i as i64),
                doc: json!({"data": "wal_test", "iteration": i}),
            }).unwrap();

            db.update_transaction(tx_id, tx).unwrap();

            // Commit triggers WAL write + fsync
            db.commit_transaction(tx_id).unwrap();
        }

        let elapsed = start.elapsed();
        let avg_nanos = elapsed.as_nanos() / iterations;

        println!("\n📊 WAL Write Performance (with fsync):");
        println!("   Total: {} commits in {:?}", iterations, elapsed);
        println!("   Average: {} per WAL write", format_duration(avg_nanos));
        println!("   Throughput: {:.0} writes/sec", iterations as f64 / elapsed.as_secs_f64());
    }

    #[test]
    fn bench_crash_recovery_time() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("bench.mlite");

        // Phase 1: Create 100 committed transactions
        {
            let db = DatabaseCore::open(&db_path).unwrap();
            db.collection("recovery_bench").unwrap();

            for i in 0..100 {
                let tx_id = db.begin_transaction();
                let mut tx = db.get_transaction(tx_id).unwrap();

                tx.add_operation(Operation::Insert {
                    collection: "recovery_bench".to_string(),
                    doc_id: DocumentId::Int(i),
                    doc: json!({"id": i}),
                }).unwrap();

                db.update_transaction(tx_id, tx).unwrap();
                db.commit_transaction(tx_id).unwrap();
            }

            // Drop db (simulate crash)
        }

        // Phase 2: Measure recovery time
        let start = Instant::now();
        {
            let _db = DatabaseCore::open(&db_path).unwrap();
            // Recovery happens in open()
        }
        let elapsed = start.elapsed();

        println!("\n📊 Crash Recovery Time (100 committed transactions):");
        println!("   Recovery time: {:?}", elapsed);
        println!("   Per transaction: {}", format_duration(elapsed.as_nanos() / 100));
    }
}
