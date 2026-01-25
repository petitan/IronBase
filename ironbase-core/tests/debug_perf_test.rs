use ironbase_core::DatabaseCore;
use serde_json::json;
use std::collections::HashMap;
use tempfile::TempDir;

fn user_doc(id: i64, name: &str, age: i64, city: &str) -> HashMap<String, serde_json::Value> {
    let mut doc = HashMap::new();
    doc.insert("_id".to_string(), json!(id));
    doc.insert("name".to_string(), json!(name));
    doc.insert("age".to_string(), json!(age));
    doc.insert("city".to_string(), json!(city));
    doc
}

#[test]
#[ignore] // Manual debug test - opens production database which may be locked
fn debug_test_large_index_reopen() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");

    eprintln!("=== PHASE 1: Create large dataset with index ===");
    {
        let db = DatabaseCore::open(&db_path).unwrap();
        eprintln!("Phase 1: DB opened");
        
        for i in 1..=1000 {
            db.insert_one("large", user_doc(i, &format!("User{}", i), i % 100, "City"))
                .unwrap();
        }
        eprintln!("Phase 1: 1000 documents inserted");

        let coll = db.collection("large").unwrap();
        coll.create_index("age".to_string(), false, false).unwrap();
        eprintln!("Phase 1: Index created");
        
        let count = db.count_documents("large", &json!({})).unwrap();
        eprintln!("Phase 1: count = {}", count);

        eprintln!("Phase 1: Closing DB...");
        db.close().unwrap();
        eprintln!("Phase 1: DB closed successfully");
    }

    // Check file exists
    eprintln!("\n=== Checking files ===");
    eprintln!("DB file exists: {}", db_path.exists());
    eprintln!("DB file size: {} bytes", std::fs::metadata(&db_path).map(|m| m.len()).unwrap_or(0));

    eprintln!("\n=== PHASE 2: Reopen and query ===");
    {
        eprintln!("Phase 2: Opening DB...");
        let db = DatabaseCore::open(&db_path).unwrap();
        eprintln!("Phase 2: DB opened");

        let count = db.count_documents("large", &json!({})).unwrap();
        eprintln!("Phase 2: count = {}", count);

        let coll = db.collection("large").unwrap();
        let age_50 = coll.find(&json!({"age": 50})).unwrap();
        eprintln!("Phase 2: age=50 query returns {} results", age_50.len());
        
        assert!(!age_50.is_empty(), "age=50 query should return results");

        db.close().unwrap();
    }
    
    eprintln!("\n=== TEST COMPLETE ===");
}
