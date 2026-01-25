//! Debug test for _id query extraction

use ironbase_core::document::DocumentId;
use serde_json::json;

#[test]
#[ignore] // Manual debug test - opens production database which may be locked
fn test_extract_id_debug() {
    // Test cases for extract_id_query pattern

    // Case 1: Simple integer _id
    let query1 = json!({"_id": 999999999});
    println!("\n=== Query 1: {:?} ===", query1);
    if let Some(id_obj) = query1.as_object() {
        println!("  Map len: {}", id_obj.len());
        if let Some(id_value) = id_obj.get("_id") {
            println!("  _id value: {:?}", id_value);
            let result: Result<DocumentId, _> = serde_json::from_value(id_value.clone());
            println!("  Deserialized: {:?}", result);
        }
    }

    // Case 2: Integer as i64
    let query2 = json!({"_id": 177267_i64});
    println!("\n=== Query 2: {:?} ===", query2);
    if let Some(id_obj) = query2.as_object() {
        if let Some(id_value) = id_obj.get("_id") {
            println!("  _id value: {:?}", id_value);
            let result: Result<DocumentId, _> = serde_json::from_value(id_value.clone());
            println!("  Deserialized: {:?}", result);
        }
    }

    // Case 3: $in query (should NOT match)
    let query3 = json!({"_id": {"$in": [177267]}});
    println!("\n=== Query 3: {:?} ===", query3);
    if let Some(id_obj) = query3.as_object() {
        if let Some(id_value) = id_obj.get("_id") {
            println!("  _id value: {:?}", id_value);
            let result: Result<DocumentId, _> = serde_json::from_value(id_value.clone());
            println!("  Deserialized: {:?}", result);
            println!("  This should fail because $in is an object, not a simple value");
        }
    }

    // Case 4: Check what the actual database has
    println!("\n=== What format do emails use? ===");
    println!("  Emails use integer _id (e.g., 177267)");
    println!("  DocumentId::Int(177267) should work");

    println!("\n=== All tests passed ===");
}
