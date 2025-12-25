// Quick benchmark comparing Safe vs Batch mode
use ironbase_core::{DatabaseCore, DurabilityMode};
use std::collections::HashMap;
use serde_json::json;
use std::time::Instant;
use tempfile::TempDir;

fn main() {
    let count = 1000;
    
    // Test 1: Safe mode (fsync every insert)
    {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("safe.mlite");
        let db = DatabaseCore::open_with_durability(&db_path, DurabilityMode::Safe).unwrap();
        
        let start = Instant::now();
        for i in 0..count {
            let mut fields = HashMap::new();
            fields.insert("name".to_string(), json!(format!("User{}", i)));
            fields.insert("age".to_string(), json!(i % 100));
            db.insert_one("users", fields).unwrap();
        }
        let elapsed = start.elapsed();
        println!("Safe mode:  {} inserts in {:?} ({:.0} ops/sec)", 
            count, elapsed, count as f64 / elapsed.as_secs_f64());
    }
    
    // Test 2: Batch mode (fsync every 100 inserts)
    {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("batch.mlite");
        let db = DatabaseCore::open_with_durability(&db_path, DurabilityMode::Batch { batch_size: 100 }).unwrap();
        
        let start = Instant::now();
        for i in 0..count {
            let mut fields = HashMap::new();
            fields.insert("name".to_string(), json!(format!("User{}", i)));
            fields.insert("age".to_string(), json!(i % 100));
            db.insert_one("users", fields).unwrap();
        }
        let elapsed = start.elapsed();
        println!("Batch mode: {} inserts in {:?} ({:.0} ops/sec)", 
            count, elapsed, count as f64 / elapsed.as_secs_f64());
    }
    
    // Test 3: Unsafe mode (no fsync)
    {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("unsafe.mlite");
        let db = DatabaseCore::open_with_durability(&db_path, DurabilityMode::unsafe_manual()).unwrap();
        
        let start = Instant::now();
        for i in 0..count {
            let mut fields = HashMap::new();
            fields.insert("name".to_string(), json!(format!("User{}", i)));
            fields.insert("age".to_string(), json!(i % 100));
            db.insert_one("users", fields).unwrap();
        }
        db.checkpoint().unwrap();
        let elapsed = start.elapsed();
        println!("Unsafe mode: {} inserts in {:?} ({:.0} ops/sec)", 
            count, elapsed, count as f64 / elapsed.as_secs_f64());
    }
}
