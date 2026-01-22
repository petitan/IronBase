//! Integration tests for upsert functionality

use ironbase_core::{storage::MemoryStorage, DatabaseCore, UpdateOptions};
use serde_json::json;

#[test]
fn test_upsert_insert_when_no_match() {
    let db = DatabaseCore::<MemoryStorage>::open_memory().unwrap();

    let options = UpdateOptions::new().with_upsert(true);
    let result = db
        .update_one_with_options(
            "users",
            &json!({"email": "new@example.com"}),
            &json!({"$set": {"name": "New User", "status": "active"}}),
            options,
        )
        .unwrap();

    // Should have inserted (upserted_id is Some)
    assert_eq!(result.matched_count, 0);
    assert_eq!(result.modified_count, 0);
    assert!(result.upserted_id.is_some());

    // Verify the document was created with correct fields
    let coll = db.collection("users").unwrap();
    let docs = coll
        .find_with_options(&json!({}), Default::default())
        .unwrap();
    assert_eq!(docs.len(), 1);

    let doc = &docs[0];
    assert_eq!(doc.get("email"), Some(&json!("new@example.com")));
    assert_eq!(doc.get("name"), Some(&json!("New User")));
    assert_eq!(doc.get("status"), Some(&json!("active")));
}

#[test]
fn test_upsert_update_when_match_exists() {
    let db = DatabaseCore::<MemoryStorage>::open_memory().unwrap();

    // First insert a document
    let mut doc = std::collections::HashMap::new();
    doc.insert("email".to_string(), json!("existing@example.com"));
    doc.insert("name".to_string(), json!("Existing User"));
    doc.insert("status".to_string(), json!("pending"));
    db.insert_one("users", doc).unwrap();

    // Now do upsert - should update existing
    let options = UpdateOptions::new().with_upsert(true);
    let result = db
        .update_one_with_options(
            "users",
            &json!({"email": "existing@example.com"}),
            &json!({"$set": {"status": "active", "lastLogin": "2024-01-01"}}),
            options,
        )
        .unwrap();

    // Should have updated (no upserted_id)
    assert_eq!(result.matched_count, 1);
    assert_eq!(result.modified_count, 1);
    assert!(result.upserted_id.is_none());

    // Verify the document was updated
    let coll = db.collection("users").unwrap();
    let docs = coll
        .find_with_options(
            &json!({"email": "existing@example.com"}),
            Default::default(),
        )
        .unwrap();
    assert_eq!(docs.len(), 1);

    let doc = &docs[0];
    assert_eq!(doc.get("status"), Some(&json!("active")));
    assert_eq!(doc.get("lastLogin"), Some(&json!("2024-01-01")));
    // Original field should be preserved
    assert_eq!(doc.get("name"), Some(&json!("Existing User")));
}

#[test]
fn test_upsert_false_no_insert_when_no_match() {
    let db = DatabaseCore::<MemoryStorage>::open_memory().unwrap();

    // Upsert disabled (default)
    let options = UpdateOptions::new().with_upsert(false);
    let result = db.update_one_with_options(
        "users",
        &json!({"email": "nonexistent@example.com"}),
        &json!({"$set": {"name": "Should Not Insert"}}),
        options,
    );

    // When upsert=false and collection doesn't exist, should return CollectionNotFound
    // This is correct MongoDB behavior - update without upsert doesn't create collection
    match result {
        Ok(r) => {
            // If somehow a collection existed, verify no match
            assert_eq!(r.matched_count, 0);
            assert_eq!(r.modified_count, 0);
            assert!(r.upserted_id.is_none());
        }
        Err(ironbase_core::IronBaseError::CollectionNotFound(_)) => {
            // This is expected - collection doesn't exist and upsert=false
        }
        Err(e) => {
            panic!("Unexpected error: {:?}", e);
        }
    }

    // Verify no documents exist (check that upsert didn't happen)
    // Note: db.collection() creates empty collection, so we check it's empty
    let collections = db.list_collections();
    // The collection should not be in the list (was never created via insert)
    assert!(
        !collections.contains(&"users".to_string()),
        "Collection 'users' should not have been created"
    );
}

#[test]
fn test_upsert_with_eq_operator() {
    let db = DatabaseCore::<MemoryStorage>::open_memory().unwrap();

    let options = UpdateOptions::new().with_upsert(true);
    let result = db
        .update_one_with_options(
            "users",
            &json!({"status": {"$eq": "active"}, "email": "user@test.com"}),
            &json!({"$set": {"name": "Test User"}}),
            options,
        )
        .unwrap();

    assert!(result.upserted_id.is_some());

    // Verify the document includes fields from $eq
    let coll = db.collection("users").unwrap();
    let docs = coll
        .find_with_options(&json!({}), Default::default())
        .unwrap();
    assert_eq!(docs.len(), 1);

    let doc = &docs[0];
    assert_eq!(doc.get("status"), Some(&json!("active")));
    assert_eq!(doc.get("email"), Some(&json!("user@test.com")));
    assert_eq!(doc.get("name"), Some(&json!("Test User")));
}

#[test]
fn test_upsert_with_nested_fields() {
    let db = DatabaseCore::<MemoryStorage>::open_memory().unwrap();

    let options = UpdateOptions::new().with_upsert(true);
    let result = db
        .update_one_with_options(
            "users",
            &json!({"user.email": "nested@example.com"}),
            &json!({"$set": {"user.name": "Nested User", "active": true}}),
            options,
        )
        .unwrap();

    assert!(result.upserted_id.is_some());

    // Verify the nested structure was created correctly
    let coll = db.collection("users").unwrap();
    let docs = coll
        .find_with_options(&json!({}), Default::default())
        .unwrap();
    assert_eq!(docs.len(), 1);

    let doc = &docs[0];
    let user = doc.get("user").unwrap();
    assert_eq!(user.get("email"), Some(&json!("nested@example.com")));
    assert_eq!(user.get("name"), Some(&json!("Nested User")));
    assert_eq!(doc.get("active"), Some(&json!(true)));
}

#[test]
fn test_upsert_with_and_operator() {
    let db = DatabaseCore::<MemoryStorage>::open_memory().unwrap();

    let options = UpdateOptions::new().with_upsert(true);
    let result = db
        .update_one_with_options(
            "users",
            &json!({
                "$and": [
                    {"email": "and@example.com"},
                    {"status": "active"}
                ]
            }),
            &json!({"$set": {"name": "And User"}}),
            options,
        )
        .unwrap();

    assert!(result.upserted_id.is_some());

    // Verify both fields from $and were included
    let coll = db.collection("users").unwrap();
    let docs = coll
        .find_with_options(&json!({}), Default::default())
        .unwrap();
    assert_eq!(docs.len(), 1);

    let doc = &docs[0];
    assert_eq!(doc.get("email"), Some(&json!("and@example.com")));
    assert_eq!(doc.get("status"), Some(&json!("active")));
    assert_eq!(doc.get("name"), Some(&json!("And User")));
}

#[test]
fn test_upsert_with_inc_creates_initial_value() {
    let db = DatabaseCore::<MemoryStorage>::open_memory().unwrap();

    let options = UpdateOptions::new().with_upsert(true);
    let result = db
        .update_one_with_options(
            "counters",
            &json!({"name": "page_views"}),
            &json!({"$inc": {"count": 1}}),
            options,
        )
        .unwrap();

    assert!(result.upserted_id.is_some());

    // Verify counter was initialized
    let coll = db.collection("counters").unwrap();
    let docs = coll
        .find_with_options(&json!({}), Default::default())
        .unwrap();
    assert_eq!(docs.len(), 1);

    let doc = &docs[0];
    assert_eq!(doc.get("name"), Some(&json!("page_views")));
    assert_eq!(doc.get("count"), Some(&json!(1)));
}

#[test]
fn test_upsert_with_push_creates_array() {
    let db = DatabaseCore::<MemoryStorage>::open_memory().unwrap();

    let options = UpdateOptions::new().with_upsert(true);
    let result = db
        .update_one_with_options(
            "users",
            &json!({"name": "Alice"}),
            &json!({"$push": {"tags": "new"}}),
            options,
        )
        .unwrap();

    assert!(result.upserted_id.is_some());

    // Verify array was created
    let coll = db.collection("users").unwrap();
    let docs = coll
        .find_with_options(&json!({}), Default::default())
        .unwrap();
    assert_eq!(docs.len(), 1);

    let doc = &docs[0];
    assert_eq!(doc.get("name"), Some(&json!("Alice")));
    assert_eq!(doc.get("tags"), Some(&json!(["new"])));
}

#[test]
fn test_upsert_returns_correct_document_id() {
    let db = DatabaseCore::<MemoryStorage>::open_memory().unwrap();

    let options = UpdateOptions::new().with_upsert(true);
    let result = db
        .update_one_with_options(
            "users",
            &json!({"email": "id-test@example.com"}),
            &json!({"$set": {"name": "ID Test"}}),
            options,
        )
        .unwrap();

    let upserted_id = result.upserted_id.unwrap();

    // Find by the returned ID
    let coll = db.collection("users").unwrap();
    let id_value = match &upserted_id {
        ironbase_core::DocumentId::Int(i) => json!(i),
        ironbase_core::DocumentId::String(s) => json!(s),
        ironbase_core::DocumentId::ObjectId(s) => json!(s),
    };

    let docs = coll
        .find_with_options(&json!({"_id": id_value}), Default::default())
        .unwrap();
    assert_eq!(docs.len(), 1);
    assert_eq!(docs[0].get("email"), Some(&json!("id-test@example.com")));
}

#[test]
fn test_upsert_ignores_comparison_operators() {
    let db = DatabaseCore::<MemoryStorage>::open_memory().unwrap();

    // Filter with comparison operators that should be ignored
    let options = UpdateOptions::new().with_upsert(true);
    let result = db
        .update_one_with_options(
            "users",
            &json!({
                "email": "valid@example.com",
                "age": {"$gt": 18, "$lt": 65}  // Should be ignored
            }),
            &json!({"$set": {"name": "Comparison Test"}}),
            options,
        )
        .unwrap();

    assert!(result.upserted_id.is_some());

    // Verify age was NOT included (comparison operators ignored)
    let coll = db.collection("users").unwrap();
    let docs = coll
        .find_with_options(&json!({}), Default::default())
        .unwrap();
    assert_eq!(docs.len(), 1);

    let doc = &docs[0];
    assert_eq!(doc.get("email"), Some(&json!("valid@example.com")));
    assert!(doc.get("age").is_none()); // Age should not be present
    assert_eq!(doc.get("name"), Some(&json!("Comparison Test")));
}
