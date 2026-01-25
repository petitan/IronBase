//! Debug test for fast path _id detection

use ironbase_core::document::DocumentId;
use ironbase_core::storage::StorageEngine;
use ironbase_core::DatabaseCore;
use serde_json::{json, Value};

/// Replicate extract_id_query logic
fn extract_id_query(query_json: &Value) -> Option<DocumentId> {
    if let Value::Object(map) = query_json {
        if map.len() == 1 {
            if let Some(id_value) = map.get("_id") {
                return serde_json::from_value(id_value.clone()).ok();
            }
        }
    }
    None
}

#[test]
#[ignore] // Manual debug test - opens production database which may be locked
fn test_fast_path_detection() {
    // Open database
    let db = DatabaseCore::<StorageEngine>::open(
        "/home/petitan/MongoLite/mcp-server/ironbase_data.mlite",
    )
    .expect("Failed to open database");

    let coll = db.collection("emails").unwrap();

    // Get a real document from the database
    let docs = coll
        .find_with_options(&json!({}), ironbase_core::FindOptions::new().with_limit(1))
        .unwrap();
    if docs.is_empty() {
        println!("No documents in collection!");
        return;
    }

    let doc = &docs[0];
    println!("\n=== Sample document ===");
    println!("_id field: {:?}", doc.get("_id"));

    // Get the _id value
    if let Some(id_val) = doc.get("_id") {
        println!(
            "_id type: {}",
            match id_val {
                Value::Number(n) => format!(
                    "Number (i64: {:?}, u64: {:?}, f64: {:?})",
                    n.as_i64(),
                    n.as_u64(),
                    n.as_f64()
                ),
                Value::String(_) => "String".to_string(),
                _ => format!("{:?}", id_val),
            }
        );

        // Build query
        let query = json!({"_id": id_val.clone()});
        println!("\n=== Query: {:?} ===", query);

        // Test extract_id_query
        let extracted = extract_id_query(&query);
        println!("extract_id_query result: {:?}", extracted);

        if extracted.is_none() {
            println!("WARNING: Fast path would NOT be used!");

            // Try direct deserialization
            let deser_result: Result<DocumentId, _> = serde_json::from_value(id_val.clone());
            println!("Direct deserialization: {:?}", deser_result);
        }
    }

    println!("\n=== Test complete ===");
}
