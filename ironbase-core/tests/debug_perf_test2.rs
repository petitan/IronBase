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
fn debug_test_large_index_reopen2() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");

    eprintln!("=== PHASE 1: Create large dataset with index ===");
    {
        let db = DatabaseCore::open(&db_path).unwrap();
        
        for i in 1..=1000 {
            db.insert_one("large", user_doc(i, &format!("User{}", i), i % 100, "City"))
                .unwrap();
        }
        eprintln!("Phase 1: 1000 documents inserted");

        let coll = db.collection("large").unwrap();
        coll.create_index("age".to_string(), false, false).unwrap();
        eprintln!("Phase 1: Index created");
        
        // Test query before close
        let age_50 = coll.find(&json!({"age": 50})).unwrap();
        eprintln!("Phase 1: age=50 BEFORE close = {} results", age_50.len());

        db.close().unwrap();
        eprintln!("Phase 1: DB closed");
    }

    // List all files in temp dir
    eprintln!("\n=== Files after Phase 1 ===");
    for entry in std::fs::read_dir(temp_dir.path()).unwrap() {
        let entry = entry.unwrap();
        let meta = entry.metadata().unwrap();
        eprintln!("  {} ({} bytes)", entry.path().display(), meta.len());
    }

    eprintln!("\n=== PHASE 2: Reopen and query ===");
    {
        let db = DatabaseCore::open(&db_path).unwrap();

        let count = db.count_documents("large", &json!({})).unwrap();
        eprintln!("Phase 2: document count = {}", count);

        let coll = db.collection("large").unwrap();
        
        // List indexes
        let indexes = coll.list_indexes().unwrap();
        eprintln!("Phase 2: indexes = {:?}", indexes);
        
        // Try explain to see query plan
        let plan = coll.explain(&json!({"age": 50})).unwrap();
        eprintln!("Phase 2: explain = {:?}", plan);
        
        // The actual query
        let age_50 = coll.find(&json!({"age": 50})).unwrap();
        eprintln!("Phase 2: age=50 query returns {} results", age_50.len());
        
        // Try a collection scan query (no index)
        let results_scan = coll.find(&json!({"name": "User50"})).unwrap();
        eprintln!("Phase 2: name=User50 (scan) = {} results", results_scan.len());
        
        // Check if documents with age=50 exist
        let all_docs = coll.find(&json!({})).unwrap();
        let with_age_50 = all_docs.iter().filter(|d| d["age"] == 50).count();
        eprintln!("Phase 2: docs with age=50 via scan = {}", with_age_50);

        db.close().unwrap();
    }
    
    eprintln!("\n=== TEST COMPLETE ===");
}
