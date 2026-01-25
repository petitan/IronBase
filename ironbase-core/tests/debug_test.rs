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
fn debug_test_index_insert_after_reopen() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");

    eprintln!("=== PHASE 1: Create and insert ===");
    {
        let db = DatabaseCore::open(&db_path).unwrap();
        for i in 1..=10 {
            db.insert_one("users", user_doc(i, &format!("User{}", i), 25, "City")).unwrap();
        }
        let coll = db.collection("users").unwrap();
        coll.create_index("age".to_string(), false, false).unwrap();
        
        let count = db.count_documents("users", &json!({})).unwrap();
        eprintln!("Phase 1: {} documents", count);
        
        let indexes = coll.list_indexes().unwrap();
        eprintln!("Phase 1: {} indexes", indexes.len());
        
        eprintln!("Phase 1: Closing DB...");
        db.close().unwrap();
        eprintln!("Phase 1: DB closed");
    }

    eprintln!("\n=== PHASE 2: Reopen and insert more ===");
    {
        eprintln!("Phase 2: Opening DB...");
        let db = DatabaseCore::open(&db_path).unwrap();
        eprintln!("Phase 2: DB opened");
        
        let count_before = db.count_documents("users", &json!({})).unwrap();
        eprintln!("Phase 2: {} documents before insert", count_before);
        
        for i in 11..=20 {
            db.insert_one("users", user_doc(i, &format!("User{}", i), 30, "City")).unwrap();
        }
        
        let count_after = db.count_documents("users", &json!({})).unwrap();
        eprintln!("Phase 2: {} documents after insert", count_after);
        
        let coll = db.collection("users").unwrap();
        let age30 = coll.find(&json!({"age": 30})).unwrap();
        eprintln!("Phase 2: age=30 query returns {} results", age30.len());
        
        eprintln!("Phase 2: Closing DB...");
        db.close().unwrap();
        eprintln!("Phase 2: DB closed");
    }

    eprintln!("\n=== PHASE 3: Verify ===");
    {
        eprintln!("Phase 3: Opening DB...");
        let db = DatabaseCore::open(&db_path).unwrap();
        eprintln!("Phase 3: DB opened");
        
        let count = db.count_documents("users", &json!({})).unwrap();
        eprintln!("Phase 3: {} documents total", count);
        
        let coll = db.collection("users").unwrap();
        let age25 = coll.find(&json!({"age": 25})).unwrap();
        eprintln!("Phase 3: age=25 query returns {} results", age25.len());
        
        let age30 = coll.find(&json!({"age": 30})).unwrap();
        eprintln!("Phase 3: age=30 query returns {} results", age30.len());
        
        assert!(!age30.is_empty(), "age=30 should have results!");
        
        db.close().unwrap();
    }
}
