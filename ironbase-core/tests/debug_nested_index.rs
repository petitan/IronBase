use ironbase_core::{storage::StorageEngine, DatabaseCore};
use serde_json::json;
use std::collections::HashMap;
use tempfile::TempDir;

#[test]
fn debug_nested_index_persistence() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("debug_nested.mlite");

    println!("\n=== SESSION 1: Create index and insert data ===");
    {
        let db = DatabaseCore::<StorageEngine>::open(&db_path).unwrap();
        let coll = db.collection("users").unwrap();

        // Create index on nested field
        coll.create_index("profile.score".to_string(), false)
            .unwrap();
        println!("Index created on profile.score");

        // List indexes
        let indexes = coll.list_indexes();
        println!("Indexes after creation: {:?}", indexes);

        // Insert 10 docs (simpler)
        for i in 0..10 {
            let score = 900 + i * 10; // 900, 910, 920, ... 990
            coll.insert_one(HashMap::from([
                ("name".to_string(), json!(format!("User{}", i))),
                (
                    "profile".to_string(),
                    json!({
                        "score": score,
                        "level": "senior"
                    }),
                ),
            ]))
            .unwrap();
            println!("Inserted user with score {}", score);
        }

        // Query before close - should work
        let results = coll.find(&json!({"profile.score": {"$gte": 900}})).unwrap();
        println!("Results BEFORE close: {} docs", results.len());

        // Also try all docs
        let all = coll.find(&json!({})).unwrap();
        println!("Total docs BEFORE close: {}", all.len());
    }
    println!("=== DB closed ===\n");

    println!("=== SESSION 2: Reopen and query ===");
    {
        let db = DatabaseCore::<StorageEngine>::open(&db_path).unwrap();
        let coll = db.collection("users").unwrap();

        // List indexes after reopen
        let indexes = coll.list_indexes();
        println!("Indexes after reopen: {:?}", indexes);

        // Try to find ALL docs first (no filter)
        let all = coll.find(&json!({})).unwrap();
        println!("Total docs after reopen: {}", all.len());

        if !all.is_empty() {
            println!(
                "First doc sample: {}",
                serde_json::to_string_pretty(&all[0]).unwrap()
            );
        }

        // Now try with filter
        let results = coll.find(&json!({"profile.score": {"$gte": 900}})).unwrap();
        println!("Results with profile.score >= 900: {} docs", results.len());

        // Try explain to see query plan
        let explain = coll
            .explain(&json!({"profile.score": {"$gte": 900}}))
            .unwrap();
        println!(
            "Explain: {}",
            serde_json::to_string_pretty(&explain).unwrap()
        );

        assert_eq!(all.len(), 10, "Should have 10 docs after reopen");
        assert_eq!(results.len(), 10, "Should find 10 users with score >= 900");
    }
}
