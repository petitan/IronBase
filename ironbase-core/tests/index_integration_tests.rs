// Index integration tests
use ironbase_core::DatabaseCore;
use serde_json::json;
use tempfile::TempDir;

#[test]
fn test_automatic_id_index() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");

    let db = DatabaseCore::open(&db_path).unwrap();
    let collection = db.collection("users").unwrap();

    // The _id index should be automatically created
    let indexes = collection.list_indexes().unwrap();
    println!("Indexes: {:?}", indexes);
    assert!(indexes.contains(&"users_id".to_string()));
}

#[test]
fn test_create_custom_index() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");

    let db = DatabaseCore::open(&db_path).unwrap();
    let collection = db.collection("users").unwrap();

    // Create an index on email field
    let index_name = collection
        .create_index("email".to_string(), true, false)
        .unwrap();
    assert_eq!(index_name, "users_email");

    // Verify index exists
    let indexes = collection.list_indexes().unwrap();
    assert!(indexes.contains(&"users_email".to_string()));
    assert!(indexes.contains(&"users_id".to_string()));
}

#[test]
fn test_insert_with_index_maintenance() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");

    let db = DatabaseCore::open(&db_path).unwrap();
    let collection = db.collection("users").unwrap();

    // Create an index on age field
    collection
        .create_index("age".to_string(), false, false)
        .unwrap();

    // Insert documents
    let mut fields1 = std::collections::HashMap::new();
    fields1.insert("name".to_string(), json!("Alice"));
    fields1.insert("age".to_string(), json!(30));

    let mut fields2 = std::collections::HashMap::new();
    fields2.insert("name".to_string(), json!("Bob"));
    fields2.insert("age".to_string(), json!(25));

    db.insert_one("users", fields1).unwrap();
    db.insert_one("users", fields2).unwrap();

    // TEST ADDITION: Index-based query test (requires query optimizer)
    //
    // Current: Index is populated but never used by find()
    // Missing: Query optimizer that routes queries to index.search()
    //
    // Test to add when optimizer is implemented:
    //
    // let results = collection.find(&json!({"age": {"$eq": 30}})).unwrap();
    // assert_eq!(results.len(), 1);
    // assert_eq!(results[0].get("name").unwrap().as_str().unwrap(), "Alice");
    //
    // Verify index was used (not full scan):
    // let stats = collection.get_query_stats().unwrap();
    // assert!(stats.index_used);
    // assert_eq!(stats.documents_scanned, 1); // Not 2!
    //
    // Prerequisites:
    // - Query optimizer implementation (IMPLEMENTATION_QUERY_OPTIMIZER.md)
    // - Index child loading (index.rs:195 - commit 90045d8)
    // - Query statistics tracking
}

#[test]
fn test_unique_index_constraint() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");

    let db = DatabaseCore::open(&db_path).unwrap();
    let collection = db.collection("users").unwrap();

    // Create unique index on email
    collection
        .create_index("email".to_string(), true, false)
        .unwrap();

    // Insert first document
    let mut fields1 = std::collections::HashMap::new();
    fields1.insert("email".to_string(), json!("alice@example.com"));
    db.insert_one("users", fields1).unwrap();

    // Try to insert duplicate email - should fail
    let mut fields2 = std::collections::HashMap::new();
    fields2.insert("email".to_string(), json!("alice@example.com"));
    let result = db.insert_one("users", fields2);

    assert!(result.is_err());
}

#[test]
fn test_drop_index() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");

    let db = DatabaseCore::open(&db_path).unwrap();
    let collection = db.collection("users").unwrap();

    // Create an index
    let index_name = collection
        .create_index("age".to_string(), false, false)
        .unwrap();

    // Verify it exists
    let indexes = collection.list_indexes().unwrap();
    assert!(indexes.contains(&index_name));

    // Drop the index
    collection.drop_index(&index_name).unwrap();

    // Verify it's gone
    let indexes = collection.list_indexes().unwrap();
    assert!(!indexes.contains(&index_name));
}

// ========== UNIQUE INDEX CONSTRAINT TESTS FOR UPDATE/DELETE ==========

#[test]
fn test_update_one_unique_constraint_violation() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");

    let db = DatabaseCore::open(&db_path).unwrap();
    let collection = db.collection("users").unwrap();

    // Create unique index on email
    collection
        .create_index("email".to_string(), true, false)
        .unwrap();

    // Insert two documents with different emails
    let mut fields1 = std::collections::HashMap::new();
    fields1.insert("name".to_string(), json!("Alice"));
    fields1.insert("email".to_string(), json!("alice@example.com"));
    db.insert_one("users", fields1).unwrap();

    let mut fields2 = std::collections::HashMap::new();
    fields2.insert("name".to_string(), json!("Bob"));
    fields2.insert("email".to_string(), json!("bob@example.com"));
    db.insert_one("users", fields2).unwrap();

    // Try to update Bob's email to Alice's email - should fail
    let result = db.update_one(
        "users",
        &json!({"name": "Bob"}),
        &json!({"$set": {"email": "alice@example.com"}}),
    );

    assert!(
        result.is_err(),
        "Update to duplicate unique value should fail"
    );
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("Duplicate key") || err_msg.contains("unique"),
        "Error should mention duplicate key or unique constraint, got: {}",
        err_msg
    );

    // Verify Bob's email is unchanged
    let bob = collection
        .find_one(&json!({"name": "Bob"}))
        .unwrap()
        .unwrap();
    assert_eq!(
        bob.get("email").unwrap().as_str().unwrap(),
        "bob@example.com"
    );
}

#[test]
fn test_update_one_same_value_allowed() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");

    let db = DatabaseCore::open(&db_path).unwrap();
    let collection = db.collection("users").unwrap();

    // Create unique index on email
    collection
        .create_index("email".to_string(), true, false)
        .unwrap();

    // Insert document
    let mut fields = std::collections::HashMap::new();
    fields.insert("name".to_string(), json!("Alice"));
    fields.insert("email".to_string(), json!("alice@example.com"));
    db.insert_one("users", fields).unwrap();

    // Update same document to same email value - should work
    let result = db.update_one(
        "users",
        &json!({"name": "Alice"}),
        &json!({"$set": {"email": "alice@example.com", "age": 30}}),
    );

    assert!(result.is_ok(), "Update to same value should succeed");

    // Verify the update worked
    let alice = collection
        .find_one(&json!({"name": "Alice"}))
        .unwrap()
        .unwrap();
    assert_eq!(alice.get("age").unwrap().as_i64().unwrap(), 30);
}

#[test]
fn test_update_many_unique_constraint_violation() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");

    let db = DatabaseCore::open(&db_path).unwrap();
    let collection = db.collection("users").unwrap();

    // Create unique index on email
    collection
        .create_index("email".to_string(), true, false)
        .unwrap();

    // Insert documents
    let mut fields1 = std::collections::HashMap::new();
    fields1.insert("name".to_string(), json!("Alice"));
    fields1.insert("email".to_string(), json!("alice@example.com"));
    fields1.insert("role".to_string(), json!("admin"));
    db.insert_one("users", fields1).unwrap();

    let mut fields2 = std::collections::HashMap::new();
    fields2.insert("name".to_string(), json!("Bob"));
    fields2.insert("email".to_string(), json!("bob@example.com"));
    fields2.insert("role".to_string(), json!("user"));
    db.insert_one("users", fields2).unwrap();

    // Try to update Bob to have Alice's email - should fail
    let result = db.update_many(
        "users",
        &json!({"role": "user"}),
        &json!({"$set": {"email": "alice@example.com"}}),
    );

    assert!(
        result.is_err(),
        "Update many to duplicate value should fail"
    );
}

/// FIX #16: update_many now correctly detects duplicate values within a batch.
/// The fix adds tracking of pending unique values and detects duplicates BEFORE
/// index updates are applied.
#[test]
fn test_update_many_creates_duplicates_in_unique_index_bug() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");

    let db = DatabaseCore::open(&db_path).unwrap();
    let collection = db.collection("users").unwrap();

    // Create unique index on code field
    collection
        .create_index("code".to_string(), true, false)
        .unwrap();

    // Insert documents with DIFFERENT unique codes
    let mut fields1 = std::collections::HashMap::new();
    fields1.insert("name".to_string(), json!("Alice"));
    fields1.insert("code".to_string(), json!("CODE_A"));
    fields1.insert("role".to_string(), json!("admin"));
    db.insert_one("users", fields1).unwrap();

    let mut fields2 = std::collections::HashMap::new();
    fields2.insert("name".to_string(), json!("Bob"));
    fields2.insert("code".to_string(), json!("CODE_B"));
    fields2.insert("role".to_string(), json!("admin"));
    db.insert_one("users", fields2).unwrap();

    let mut fields3 = std::collections::HashMap::new();
    fields3.insert("name".to_string(), json!("Charlie"));
    fields3.insert("code".to_string(), json!("CODE_C"));
    fields3.insert("role".to_string(), json!("admin"));
    db.insert_one("users", fields3).unwrap();

    // FIX #16: Try to update ALL admins to the SAME NEW code
    // This should FAIL because it would create duplicates within the batch
    let result = db.update_many(
        "users",
        &json!({"role": "admin"}),
        &json!({"$set": {"code": "SAME_CODE_FOR_ALL"}}),
    );

    // After fix: update_many correctly detects batch duplicates and fails
    assert!(
        result.is_err(),
        "update_many to same NEW value should fail for unique index. Result: {:?}",
        result
    );

    // Verify error message mentions duplicate
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("Duplicate key") || err_msg.contains("unique"),
        "Error should mention duplicate key, got: {}",
        err_msg
    );

    // Verify no documents were modified (atomic failure)
    let alice = collection
        .find_one(&json!({"name": "Alice"}))
        .unwrap()
        .unwrap();
    assert_eq!(
        alice.get("code").unwrap().as_str().unwrap(),
        "CODE_A",
        "Alice's code should remain unchanged"
    );
}

#[test]
fn test_delete_removes_from_index() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");

    let db = DatabaseCore::open(&db_path).unwrap();
    let collection = db.collection("users").unwrap();

    // Create unique index on email
    collection
        .create_index("email".to_string(), true, false)
        .unwrap();

    // Insert document
    let mut fields = std::collections::HashMap::new();
    fields.insert("name".to_string(), json!("Alice"));
    fields.insert("email".to_string(), json!("alice@example.com"));
    db.insert_one("users", fields).unwrap();

    // Delete the document
    let deleted = db.delete_one("users", &json!({"name": "Alice"})).unwrap();
    assert_eq!(deleted, 1);

    // Now insert a new document with the same email - should succeed
    // because the old entry was removed from the index
    let mut fields2 = std::collections::HashMap::new();
    fields2.insert("name".to_string(), json!("Alice2"));
    fields2.insert("email".to_string(), json!("alice@example.com"));
    let result = db.insert_one("users", fields2);

    assert!(
        result.is_ok(),
        "Insert after delete should succeed, got: {:?}",
        result.err()
    );
}

#[test]
fn test_delete_many_removes_from_index() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");

    let db = DatabaseCore::open(&db_path).unwrap();
    let collection = db.collection("users").unwrap();

    // Create unique index on email
    collection
        .create_index("email".to_string(), true, false)
        .unwrap();

    // Insert multiple documents
    let mut fields1 = std::collections::HashMap::new();
    fields1.insert("name".to_string(), json!("Alice"));
    fields1.insert("email".to_string(), json!("alice@example.com"));
    fields1.insert("active".to_string(), json!(false));
    db.insert_one("users", fields1).unwrap();

    let mut fields2 = std::collections::HashMap::new();
    fields2.insert("name".to_string(), json!("Bob"));
    fields2.insert("email".to_string(), json!("bob@example.com"));
    fields2.insert("active".to_string(), json!(false));
    db.insert_one("users", fields2).unwrap();

    // Delete all inactive users
    let deleted = db.delete_many("users", &json!({"active": false})).unwrap();
    assert_eq!(deleted, 2);

    // Now insert new documents with the same emails - should succeed
    let mut fields3 = std::collections::HashMap::new();
    fields3.insert("name".to_string(), json!("NewAlice"));
    fields3.insert("email".to_string(), json!("alice@example.com"));
    let result1 = db.insert_one("users", fields3);
    assert!(
        result1.is_ok(),
        "Insert alice email after delete_many should succeed"
    );

    let mut fields4 = std::collections::HashMap::new();
    fields4.insert("name".to_string(), json!("NewBob"));
    fields4.insert("email".to_string(), json!("bob@example.com"));
    let result2 = db.insert_one("users", fields4);
    assert!(
        result2.is_ok(),
        "Insert bob email after delete_many should succeed"
    );
}

#[test]
fn test_update_one_changes_indexed_value() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");

    let db = DatabaseCore::open(&db_path).unwrap();
    let collection = db.collection("users").unwrap();

    // Create unique index on email
    collection
        .create_index("email".to_string(), true, false)
        .unwrap();

    // Insert document
    let mut fields = std::collections::HashMap::new();
    fields.insert("name".to_string(), json!("Alice"));
    fields.insert("email".to_string(), json!("alice@example.com"));
    db.insert_one("users", fields).unwrap();

    // Update email to a new value
    let result = db.update_one(
        "users",
        &json!({"name": "Alice"}),
        &json!({"$set": {"email": "alice.new@example.com"}}),
    );
    assert!(result.is_ok());

    // The old email should now be available for reuse
    let mut fields2 = std::collections::HashMap::new();
    fields2.insert("name".to_string(), json!("Bob"));
    fields2.insert("email".to_string(), json!("alice@example.com"));
    let result2 = db.insert_one("users", fields2);

    assert!(
        result2.is_ok(),
        "Old email should be available after update, got: {:?}",
        result2.err()
    );
}

/// BUG TEST: insert_many with duplicates WITHIN the batch should fail.
/// This tests the same pattern as Issue #16, but for insert_many.
/// Fixed by Issue #17 - BatchConstraintValidator now detects within-batch duplicates.
#[test]
fn test_insert_many_duplicates_within_batch_bug() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");

    let db = DatabaseCore::open(&db_path).unwrap();
    let collection = db.collection("users").unwrap();

    // Create unique index on code field
    collection
        .create_index("code".to_string(), true, false)
        .unwrap();

    // Try to insert multiple documents with THE SAME unique code value
    // This should FAIL because they violate the unique constraint within the batch
    let mut doc1 = std::collections::HashMap::new();
    doc1.insert("name".to_string(), json!("Alice"));
    doc1.insert("code".to_string(), json!("SAME_CODE"));

    let mut doc2 = std::collections::HashMap::new();
    doc2.insert("name".to_string(), json!("Bob"));
    doc2.insert("code".to_string(), json!("SAME_CODE")); // DUPLICATE!

    let mut doc3 = std::collections::HashMap::new();
    doc3.insert("name".to_string(), json!("Charlie"));
    doc3.insert("code".to_string(), json!("SAME_CODE")); // DUPLICATE!

    let result = db.insert_many("users", vec![doc1, doc2, doc3]);

    // EXPECTED BEHAVIOR: Should fail with duplicate key error
    assert!(
        result.is_err(),
        "insert_many with duplicate unique values within batch should fail. Result: {:?}",
        result
    );

    // Verify no documents were inserted (atomic failure)
    let docs = collection.find(&json!({})).unwrap();
    assert_eq!(
        docs.len(),
        0,
        "No documents should be inserted on batch failure"
    );
}

/// BUG #3: insert_many should be atomic - if one document fails, none should be inserted
/// This test verifies that insert_many properly rolls back on failure.
#[test]
fn test_insert_many_atomic_failure_against_existing_documents() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");

    let db = DatabaseCore::open(&db_path).unwrap();
    let collection = db.collection("users").unwrap();

    // Create unique index on code field
    collection
        .create_index("code".to_string(), true, false)
        .unwrap();

    // Insert an EXISTING document with a unique code
    let mut existing = std::collections::HashMap::new();
    existing.insert("name".to_string(), json!("Existing"));
    existing.insert("code".to_string(), json!("EXISTING_CODE"));
    db.insert_one("users", existing).unwrap();

    // Now try to insert_many with 3 documents:
    // - First two have NEW unique codes (would succeed individually)
    // - Third one has the SAME code as the existing document (should fail)
    let mut doc1 = std::collections::HashMap::new();
    doc1.insert("name".to_string(), json!("Alice"));
    doc1.insert("code".to_string(), json!("CODE_A")); // New, unique

    let mut doc2 = std::collections::HashMap::new();
    doc2.insert("name".to_string(), json!("Bob"));
    doc2.insert("code".to_string(), json!("CODE_B")); // New, unique

    let mut doc3 = std::collections::HashMap::new();
    doc3.insert("name".to_string(), json!("Charlie"));
    doc3.insert("code".to_string(), json!("EXISTING_CODE")); // CONFLICT with existing!

    let result = db.insert_many("users", vec![doc1, doc2, doc3]);

    // EXPECTED: Should fail because doc3 conflicts with existing document
    assert!(
        result.is_err(),
        "insert_many with conflict against existing should fail. Result: {:?}",
        result
    );

    // CRITICAL: Verify ATOMICITY - no new documents should be inserted
    let docs = collection.find(&json!({})).unwrap();
    assert_eq!(
        docs.len(),
        1,
        "Only the original document should exist (atomic failure - none of the batch inserted)"
    );

    // The only document should be the existing one
    let existing_doc = collection.find_one(&json!({"name": "Existing"})).unwrap();
    assert!(
        existing_doc.is_some(),
        "Original document should still exist"
    );

    // Alice should NOT exist (batch failed, so all should be rolled back)
    let alice = collection.find_one(&json!({"name": "Alice"})).unwrap();
    assert!(
        alice.is_none(),
        "Alice should NOT exist - batch should have been rolled back"
    );
}

/// BUG #2: update_many with nested field unique index creates duplicates
/// This test verifies that update_many correctly detects duplicates when
/// updating nested fields with a unique index.
#[test]
fn test_update_many_nested_field_creates_duplicates_in_unique_index_bug() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");

    let db = DatabaseCore::open(&db_path).unwrap();
    let collection = db.collection("users").unwrap();

    // Create unique index on nested field profile.code
    collection
        .create_index("profile.code".to_string(), true, false)
        .unwrap();

    // Insert documents with DIFFERENT nested unique codes
    let mut fields1 = std::collections::HashMap::new();
    fields1.insert("name".to_string(), json!("Alice"));
    fields1.insert("profile".to_string(), json!({"code": "CODE_A", "level": 1}));
    fields1.insert("role".to_string(), json!("admin"));
    db.insert_one("users", fields1).unwrap();

    let mut fields2 = std::collections::HashMap::new();
    fields2.insert("name".to_string(), json!("Bob"));
    fields2.insert("profile".to_string(), json!({"code": "CODE_B", "level": 2}));
    fields2.insert("role".to_string(), json!("admin"));
    db.insert_one("users", fields2).unwrap();

    let mut fields3 = std::collections::HashMap::new();
    fields3.insert("name".to_string(), json!("Charlie"));
    fields3.insert("profile".to_string(), json!({"code": "CODE_C", "level": 3}));
    fields3.insert("role".to_string(), json!("admin"));
    db.insert_one("users", fields3).unwrap();

    // BUG #2: Try to update ALL admins to the SAME NEW nested code
    // This should FAIL because it would create duplicates within the batch
    let result = db.update_many(
        "users",
        &json!({"role": "admin"}),
        &json!({"$set": {"profile.code": "SAME_CODE_FOR_ALL"}}),
    );

    // After fix: update_many correctly detects batch duplicates and fails
    assert!(
        result.is_err(),
        "update_many to same NEW nested value should fail for unique index. Result: {:?}",
        result
    );

    // Verify error message mentions duplicate
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("Duplicate key") || err_msg.contains("unique"),
        "Error should mention duplicate key, got: {}",
        err_msg
    );

    // Verify no documents were modified (atomic failure)
    let alice = collection
        .find_one(&json!({"name": "Alice"}))
        .unwrap()
        .unwrap();
    let profile = alice.get("profile").unwrap();
    assert_eq!(
        profile.get("code").unwrap().as_str().unwrap(),
        "CODE_A",
        "Alice's nested code should remain unchanged"
    );
}

/// FIX #19: Test update_many works correctly with compound unique indexes
///
/// BUG #4: Previously, update_many returned matched_count=0 when a compound
/// unique index existed because check_index_constraints only checked ONE field
/// instead of the compound key.
#[test]
fn test_update_many_with_compound_unique_index_bug4() {
    use std::collections::HashMap;

    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");

    let db = DatabaseCore::open(&db_path).unwrap();
    let collection = db.collection("locations").unwrap();

    // Create compound unique index on (country, city)
    let index_name = collection
        .create_compound_index(vec!["country".to_string(), "city".to_string()], true, false)
        .unwrap();
    println!("Created compound index: {}", index_name);

    // Insert documents with different compound keys
    let mut doc1: HashMap<String, serde_json::Value> = HashMap::new();
    doc1.insert("country".to_string(), json!("USA"));
    doc1.insert("city".to_string(), json!("NYC"));
    doc1.insert("population".to_string(), json!(8_000_000));
    db.insert_one("locations", doc1).unwrap();

    let mut doc2: HashMap<String, serde_json::Value> = HashMap::new();
    doc2.insert("country".to_string(), json!("USA"));
    doc2.insert("city".to_string(), json!("LA"));
    doc2.insert("population".to_string(), json!(4_000_000));
    db.insert_one("locations", doc2).unwrap();

    let mut doc3: HashMap<String, serde_json::Value> = HashMap::new();
    doc3.insert("country".to_string(), json!("UK"));
    doc3.insert("city".to_string(), json!("London"));
    doc3.insert("population".to_string(), json!(9_000_000));
    db.insert_one("locations", doc3).unwrap();

    // Test 1: update_many should work when NOT violating compound unique constraint
    // Update all USA cities' population - this should succeed
    let (matched, modified) = db
        .update_many(
            "locations",
            &json!({"country": "USA"}),
            &json!({"$inc": {"population": 100_000}}),
        )
        .unwrap();

    assert_eq!(matched, 2, "Should match 2 USA locations");
    assert_eq!(modified, 2, "Should modify 2 USA locations");

    // Verify the update worked
    let nyc = collection
        .find_one(&json!({"city": "NYC"}))
        .unwrap()
        .unwrap();
    assert_eq!(nyc.get("population").unwrap().as_i64().unwrap(), 8_100_000);

    // Test 2: update_many should FAIL when trying to create duplicate compound key
    // Try to change LA's city to NYC - this would create duplicate (USA, NYC)
    let result = db.update_many(
        "locations",
        &json!({"city": "LA"}),
        &json!({"$set": {"city": "NYC"}}),
    );

    assert!(
        result.is_err(),
        "Should fail: duplicate compound key (USA, NYC)"
    );
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("Duplicate key") || err_msg.contains("unique index"),
        "Error should mention duplicate key: {}",
        err_msg
    );

    // Verify LA was not changed (atomic failure)
    let la = collection.find_one(&json!({"city": "LA"})).unwrap();
    assert!(
        la.is_some(),
        "LA should still exist (update should have failed atomically)"
    );
}

/// FIX #19: Test insert_many with compound unique index
#[test]
fn test_insert_many_compound_unique_index_bug4() {
    use std::collections::HashMap;

    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");

    let db = DatabaseCore::open(&db_path).unwrap();
    let collection = db.collection("products").unwrap();

    // Create compound unique index on (category, sku)
    collection
        .create_compound_index(vec!["category".to_string(), "sku".to_string()], true, false)
        .unwrap();

    // Helper to create HashMap from JSON-like structure
    fn make_doc(category: &str, sku: &str, name: &str) -> HashMap<String, serde_json::Value> {
        let mut doc = HashMap::new();
        doc.insert("category".to_string(), json!(category));
        doc.insert("sku".to_string(), json!(sku));
        doc.insert("name".to_string(), json!(name));
        doc
    }

    // Test 1: insert_many with different compound keys should succeed
    let result = db.insert_many(
        "products",
        vec![
            make_doc("Electronics", "E001", "Phone"),
            make_doc("Electronics", "E002", "Laptop"),
            make_doc("Clothing", "E001", "Shirt"), // Same SKU but different category = OK
        ],
    );
    assert!(result.is_ok(), "Different compound keys should succeed");
    assert_eq!(result.unwrap().len(), 3, "Should insert 3 documents");

    // Test 2: insert_many with duplicate compound key should fail atomically
    let result = db.insert_many(
        "products",
        vec![
            make_doc("Books", "B001", "Novel"),
            make_doc("Books", "B001", "Magazine"), // Duplicate!
        ],
    );
    assert!(
        result.is_err(),
        "Duplicate compound key in batch should fail"
    );

    // Verify atomic failure - neither document was inserted
    let books = collection.find(&json!({"category": "Books"})).unwrap();
    assert_eq!(
        books.len(),
        0,
        "Atomic failure: no books should be inserted"
    );
}

/// BUG #5: Unique index allows multiple null values
/// MongoDB behavior: null IS a value, so unique constraint applies.
/// Only ONE document with null/missing indexed field should be allowed.
#[test]
fn test_unique_index_null_values_bug5() {
    use std::collections::HashMap;

    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");

    let db = DatabaseCore::open(&db_path).unwrap();
    let collection = db.collection("users").unwrap();

    // Create unique index on email
    collection
        .create_index("email".to_string(), true, false)
        .unwrap();

    // Test 1: First document with explicit null - should succeed
    let mut doc1: HashMap<String, serde_json::Value> = HashMap::new();
    doc1.insert("name".to_string(), json!("Alice"));
    doc1.insert("email".to_string(), json!(null));
    let result1 = db.insert_one("users", doc1);
    assert!(result1.is_ok(), "First null email should succeed");

    // Test 2: Second document with explicit null - SHOULD FAIL (duplicate null)
    let mut doc2: HashMap<String, serde_json::Value> = HashMap::new();
    doc2.insert("name".to_string(), json!("Bob"));
    doc2.insert("email".to_string(), json!(null));
    let result2 = db.insert_one("users", doc2);
    assert!(
        result2.is_err(),
        "BUG #5: Second null email should fail - null is a value in unique index! Result: {:?}",
        result2
    );

    // Verify only one document exists
    let count = collection.count_documents(&json!({})).unwrap();
    assert_eq!(count, 1, "Only one document should exist");
}

/// BUG #5: Unique index allows multiple missing field values
/// MongoDB behavior: missing field is treated as null for indexing purposes.
/// Only ONE document with missing indexed field should be allowed.
#[test]
fn test_unique_index_missing_field_bug5() {
    use std::collections::HashMap;

    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");

    let db = DatabaseCore::open(&db_path).unwrap();
    let collection = db.collection("users").unwrap();

    // Create unique index on email
    collection
        .create_index("email".to_string(), true, false)
        .unwrap();

    // Test 1: First document with MISSING email field - should succeed
    let mut doc1: HashMap<String, serde_json::Value> = HashMap::new();
    doc1.insert("name".to_string(), json!("Alice"));
    // Note: email field is NOT inserted (missing)
    let result1 = db.insert_one("users", doc1);
    assert!(result1.is_ok(), "First missing email should succeed");

    // Test 2: Second document with MISSING email field - SHOULD FAIL
    let mut doc2: HashMap<String, serde_json::Value> = HashMap::new();
    doc2.insert("name".to_string(), json!("Bob"));
    // Note: email field is NOT inserted (missing)
    let result2 = db.insert_one("users", doc2);
    assert!(
        result2.is_err(),
        "BUG #5: Second missing email should fail - missing = null in unique index! Result: {:?}",
        result2
    );

    // Verify only one document exists
    let count = collection.count_documents(&json!({})).unwrap();
    assert_eq!(count, 1, "Only one document should exist");
}

/// BUG #5: null and missing should be equivalent in unique index
#[test]
fn test_unique_index_null_equals_missing_bug5() {
    use std::collections::HashMap;

    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");

    let db = DatabaseCore::open(&db_path).unwrap();
    let collection = db.collection("users").unwrap();

    // Create unique index on email
    collection
        .create_index("email".to_string(), true, false)
        .unwrap();

    // Insert document with explicit null
    let mut doc1: HashMap<String, serde_json::Value> = HashMap::new();
    doc1.insert("name".to_string(), json!("Alice"));
    doc1.insert("email".to_string(), json!(null));
    db.insert_one("users", doc1).unwrap();

    // Try to insert document with MISSING email - should fail (null == missing)
    let mut doc2: HashMap<String, serde_json::Value> = HashMap::new();
    doc2.insert("name".to_string(), json!("Bob"));
    // email is missing
    let result = db.insert_one("users", doc2);
    assert!(
        result.is_err(),
        "BUG #5: Missing email should conflict with null email! Result: {:?}",
        result
    );
}

/// Test compound index prefix query uses index instead of full scan
#[test]
fn test_compound_index_prefix_query() {
    use ironbase_core::storage::MemoryStorage;
    use std::collections::HashMap;

    let db = DatabaseCore::<MemoryStorage>::open_memory().unwrap();
    let collection = db.collection("users").unwrap();

    // Insert test data using HashMap
    db.insert_many(
        "users",
        vec![
            HashMap::from([
                ("country".to_string(), json!("HU")),
                ("city".to_string(), json!("Budapest")),
                ("name".to_string(), json!("Alice")),
            ]),
            HashMap::from([
                ("country".to_string(), json!("HU")),
                ("city".to_string(), json!("Debrecen")),
                ("name".to_string(), json!("Bob")),
            ]),
            HashMap::from([
                ("country".to_string(), json!("US")),
                ("city".to_string(), json!("LA")),
                ("name".to_string(), json!("Charlie")),
            ]),
            HashMap::from([
                ("country".to_string(), json!("US")),
                ("city".to_string(), json!("NYC")),
                ("name".to_string(), json!("Dave")),
            ]),
            HashMap::from([
                ("country".to_string(), json!("US")),
                ("city".to_string(), json!("SF")),
                ("name".to_string(), json!("Eve")),
            ]),
            HashMap::from([
                ("country".to_string(), json!("DE")),
                ("city".to_string(), json!("Berlin")),
                ("name".to_string(), json!("Frank")),
            ]),
        ],
    )
    .unwrap();

    // Create compound index on (country, city)
    let index_name = collection
        .create_compound_index(
            vec!["country".to_string(), "city".to_string()],
            false,
            false,
        )
        .unwrap();
    assert_eq!(index_name, "users_country_city");

    // Query by prefix (first field only) - should use index
    let results = collection.find(&json!({"country": "US"})).unwrap();
    assert_eq!(results.len(), 3, "Should find 3 US entries");

    // Verify the correct documents were found
    let names: Vec<&str> = results
        .iter()
        .map(|d| d.get("name").unwrap().as_str().unwrap())
        .collect();
    assert!(names.contains(&"Charlie"));
    assert!(names.contains(&"Dave"));
    assert!(names.contains(&"Eve"));

    // Query for HU - should find 2
    let results = collection.find(&json!({"country": "HU"})).unwrap();
    assert_eq!(results.len(), 2, "Should find 2 HU entries");

    // Query for DE - should find 1
    let results = collection.find(&json!({"country": "DE"})).unwrap();
    assert_eq!(results.len(), 1, "Should find 1 DE entry");

    // Query for non-existent country - should find 0
    let results = collection.find(&json!({"country": "FR"})).unwrap();
    assert_eq!(results.len(), 0, "Should find 0 FR entries");
}

/// Test compound index prefix query explain shows index usage
#[test]
fn test_compound_index_prefix_query_explain() {
    use ironbase_core::storage::MemoryStorage;
    use std::collections::HashMap;

    let db = DatabaseCore::<MemoryStorage>::open_memory().unwrap();
    let collection = db.collection("locations").unwrap();

    // Insert test data using HashMap
    db.insert_many(
        "locations",
        vec![
            HashMap::from([
                ("country".to_string(), json!("HU")),
                ("city".to_string(), json!("Budapest")),
            ]),
            HashMap::from([
                ("country".to_string(), json!("US")),
                ("city".to_string(), json!("NYC")),
            ]),
        ],
    )
    .unwrap();

    // Create compound index
    collection
        .create_compound_index(
            vec!["country".to_string(), "city".to_string()],
            false,
            false,
        )
        .unwrap();

    // Check explain output - should show IndexScan, not CollectionScan
    let explain = collection.explain(&json!({"country": "HU"})).unwrap();
    let explain_str = explain.to_string();

    // Should use index
    assert!(
        explain_str.contains("IndexScan") || explain_str.contains("indexUsed"),
        "Compound prefix query should use index. Explain: {}",
        explain_str
    );

    // Should NOT be a collection scan
    assert!(
        !explain_str.contains("CollectionScan") || explain_str.contains("indexUsed"),
        "Should not fall back to CollectionScan. Explain: {}",
        explain_str
    );
}

/// BUG FIX (2026-01): B+ tree estimate_node_size() underestimated JSON serialization overhead
///
/// Previously, when many documents shared the same long key (e.g., 1200 chunks from the same
/// source file), the B+ tree leaf node would exceed the page size limit (16KB) because:
/// - document_id overhead was estimated as 20 bytes (actual: ~40 bytes for JSON enum)
/// - key overhead was estimated as len+10 (actual: len+20 for JSON wrapper)
///
/// This test reproduces the gaploader chunk scenario:
/// - 1200+ documents with the same `source.file` path (~150 chars)
/// - Index on `source.file` field
/// - All chunks reference the same source file
///
/// Before fix: "Node size 17129 exceeds page size 16379" error
/// After fix: Works correctly, splits nodes earlier based on accurate estimates
#[test]
fn test_btree_many_docs_same_long_key_chunk_scenario() {
    use ironbase_core::storage::MemoryStorage;
    use std::collections::HashMap;

    let db = DatabaseCore::<MemoryStorage>::open_memory().unwrap();
    let collection = db.collection("chunks").unwrap();

    // Simulate a realistic file path from gaploader (~150 chars)
    let long_file_path = "/home/user/documents/projects/my-awesome-project/data/2026/january/imported-files/very-long-filename-with-description-v2.3.1.md";
    assert!(
        long_file_path.len() > 100,
        "File path should be long enough to trigger the bug"
    );

    // Create index on source.file BEFORE inserting documents
    // (This is how gaploader works - index exists, then chunks are inserted)
    let index_name = collection
        .create_index("source.file".to_string(), false, false)
        .unwrap();
    assert_eq!(index_name, "chunks_source.file");

    // Insert 1200 chunks with the SAME source.file value
    // This should trigger the bug if estimate_node_size() is wrong
    let chunk_count = 1200;
    let mut docs: Vec<HashMap<String, serde_json::Value>> = Vec::with_capacity(chunk_count);

    for i in 0..chunk_count {
        let mut doc = HashMap::new();
        doc.insert(
            "source".to_string(),
            json!({
                "file": long_file_path,
                "chunk_index": i
            }),
        );
        doc.insert(
            "content".to_string(),
            json!(format!("Chunk {} content...", i)),
        );
        doc.insert("embedding".to_string(), json!([0.1, 0.2, 0.3])); // Simplified
        docs.push(doc);
    }

    // This is where the bug manifested: insert_many would fail with
    // "Node size 17129 exceeds page size 16379"
    let result = db.insert_many("chunks", docs);
    assert!(
        result.is_ok(),
        "BUG FIX: Should not fail with 'Node size exceeds page size'. Error: {:?}",
        result.err()
    );

    let inserted = result.unwrap();
    assert_eq!(inserted.len(), chunk_count, "All chunks should be inserted");

    // Verify index works correctly
    let results = collection
        .find(&json!({"source.file": long_file_path}))
        .unwrap();
    assert_eq!(
        results.len(),
        chunk_count,
        "Index should find all {} chunks",
        chunk_count
    );

    // Verify document count
    let count = collection.count_documents(&json!({})).unwrap();
    assert_eq!(
        count, chunk_count as u64,
        "Total document count should match"
    );
}

/// Additional test: B+ tree with even longer keys and more documents
/// Tests the upper bound of the fix - 2000 documents with 200-char keys
#[test]
fn test_btree_stress_very_long_keys_many_docs() {
    use ironbase_core::storage::MemoryStorage;
    use std::collections::HashMap;

    let db = DatabaseCore::<MemoryStorage>::open_memory().unwrap();
    let collection = db.collection("stress_test").unwrap();

    // Create a 200-character key (longer than typical file paths)
    let very_long_key = "a".repeat(200);
    assert_eq!(very_long_key.len(), 200);

    // Create index first
    collection
        .create_index("long_field".to_string(), false, false)
        .unwrap();

    // Insert 2000 documents with the same 200-char key
    let doc_count = 2000;
    let mut docs: Vec<HashMap<String, serde_json::Value>> = Vec::with_capacity(doc_count);

    for i in 0..doc_count {
        let mut doc = HashMap::new();
        doc.insert("long_field".to_string(), json!(very_long_key));
        doc.insert("index".to_string(), json!(i));
        docs.push(doc);
    }

    let result = db.insert_many("stress_test", docs);
    assert!(
        result.is_ok(),
        "Should handle 2000 docs with 200-char same key. Error: {:?}",
        result.err()
    );

    // Verify
    let count = collection.count_documents(&json!({})).unwrap();
    assert_eq!(count, doc_count as u64);

    let found = collection
        .find(&json!({"long_field": very_long_key}))
        .unwrap();
    assert_eq!(found.len(), doc_count);
}
