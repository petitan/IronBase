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
    let indexes = collection.list_indexes();
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
    let index_name = collection.create_index("email".to_string(), true).unwrap();
    assert_eq!(index_name, "users_email");

    // Verify index exists
    let indexes = collection.list_indexes();
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
    collection.create_index("age".to_string(), false).unwrap();

    // Insert documents
    let mut fields1 = std::collections::HashMap::new();
    fields1.insert("name".to_string(), json!("Alice"));
    fields1.insert("age".to_string(), json!(30));

    let mut fields2 = std::collections::HashMap::new();
    fields2.insert("name".to_string(), json!("Bob"));
    fields2.insert("age".to_string(), json!(25));

    collection.insert_one(fields1).unwrap();
    collection.insert_one(fields2).unwrap();

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
    collection.create_index("email".to_string(), true).unwrap();

    // Insert first document
    let mut fields1 = std::collections::HashMap::new();
    fields1.insert("email".to_string(), json!("alice@example.com"));
    collection.insert_one(fields1).unwrap();

    // Try to insert duplicate email - should fail
    let mut fields2 = std::collections::HashMap::new();
    fields2.insert("email".to_string(), json!("alice@example.com"));
    let result = collection.insert_one(fields2);

    assert!(result.is_err());
}

#[test]
fn test_drop_index() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");

    let db = DatabaseCore::open(&db_path).unwrap();
    let collection = db.collection("users").unwrap();

    // Create an index
    let index_name = collection.create_index("age".to_string(), false).unwrap();

    // Verify it exists
    let indexes = collection.list_indexes();
    assert!(indexes.contains(&index_name));

    // Drop the index
    collection.drop_index(&index_name).unwrap();

    // Verify it's gone
    let indexes = collection.list_indexes();
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
    collection.create_index("email".to_string(), true).unwrap();

    // Insert two documents with different emails
    let mut fields1 = std::collections::HashMap::new();
    fields1.insert("name".to_string(), json!("Alice"));
    fields1.insert("email".to_string(), json!("alice@example.com"));
    collection.insert_one(fields1).unwrap();

    let mut fields2 = std::collections::HashMap::new();
    fields2.insert("name".to_string(), json!("Bob"));
    fields2.insert("email".to_string(), json!("bob@example.com"));
    collection.insert_one(fields2).unwrap();

    // Try to update Bob's email to Alice's email - should fail
    let result = collection.update_one(
        &json!({"name": "Bob"}),
        &json!({"$set": {"email": "alice@example.com"}}),
    );

    assert!(result.is_err(), "Update to duplicate unique value should fail");
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("Duplicate key") || err_msg.contains("unique"),
        "Error should mention duplicate key or unique constraint, got: {}",
        err_msg
    );

    // Verify Bob's email is unchanged
    let bob = collection.find_one(&json!({"name": "Bob"})).unwrap().unwrap();
    assert_eq!(bob.get("email").unwrap().as_str().unwrap(), "bob@example.com");
}

#[test]
fn test_update_one_same_value_allowed() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");

    let db = DatabaseCore::open(&db_path).unwrap();
    let collection = db.collection("users").unwrap();

    // Create unique index on email
    collection.create_index("email".to_string(), true).unwrap();

    // Insert document
    let mut fields = std::collections::HashMap::new();
    fields.insert("name".to_string(), json!("Alice"));
    fields.insert("email".to_string(), json!("alice@example.com"));
    collection.insert_one(fields).unwrap();

    // Update same document to same email value - should work
    let result = collection.update_one(
        &json!({"name": "Alice"}),
        &json!({"$set": {"email": "alice@example.com", "age": 30}}),
    );

    assert!(result.is_ok(), "Update to same value should succeed");

    // Verify the update worked
    let alice = collection.find_one(&json!({"name": "Alice"})).unwrap().unwrap();
    assert_eq!(alice.get("age").unwrap().as_i64().unwrap(), 30);
}

#[test]
fn test_update_many_unique_constraint_violation() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");

    let db = DatabaseCore::open(&db_path).unwrap();
    let collection = db.collection("users").unwrap();

    // Create unique index on email
    collection.create_index("email".to_string(), true).unwrap();

    // Insert documents
    let mut fields1 = std::collections::HashMap::new();
    fields1.insert("name".to_string(), json!("Alice"));
    fields1.insert("email".to_string(), json!("alice@example.com"));
    fields1.insert("role".to_string(), json!("admin"));
    collection.insert_one(fields1).unwrap();

    let mut fields2 = std::collections::HashMap::new();
    fields2.insert("name".to_string(), json!("Bob"));
    fields2.insert("email".to_string(), json!("bob@example.com"));
    fields2.insert("role".to_string(), json!("user"));
    collection.insert_one(fields2).unwrap();

    // Try to update Bob to have Alice's email - should fail
    let result = collection.update_many(
        &json!({"role": "user"}),
        &json!({"$set": {"email": "alice@example.com"}}),
    );

    assert!(result.is_err(), "Update many to duplicate value should fail");
}

#[test]
fn test_delete_removes_from_index() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");

    let db = DatabaseCore::open(&db_path).unwrap();
    let collection = db.collection("users").unwrap();

    // Create unique index on email
    collection.create_index("email".to_string(), true).unwrap();

    // Insert document
    let mut fields = std::collections::HashMap::new();
    fields.insert("name".to_string(), json!("Alice"));
    fields.insert("email".to_string(), json!("alice@example.com"));
    collection.insert_one(fields).unwrap();

    // Delete the document
    let deleted = collection.delete_one(&json!({"name": "Alice"})).unwrap();
    assert_eq!(deleted, 1);

    // Now insert a new document with the same email - should succeed
    // because the old entry was removed from the index
    let mut fields2 = std::collections::HashMap::new();
    fields2.insert("name".to_string(), json!("Alice2"));
    fields2.insert("email".to_string(), json!("alice@example.com"));
    let result = collection.insert_one(fields2);

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
    collection.create_index("email".to_string(), true).unwrap();

    // Insert multiple documents
    let mut fields1 = std::collections::HashMap::new();
    fields1.insert("name".to_string(), json!("Alice"));
    fields1.insert("email".to_string(), json!("alice@example.com"));
    fields1.insert("active".to_string(), json!(false));
    collection.insert_one(fields1).unwrap();

    let mut fields2 = std::collections::HashMap::new();
    fields2.insert("name".to_string(), json!("Bob"));
    fields2.insert("email".to_string(), json!("bob@example.com"));
    fields2.insert("active".to_string(), json!(false));
    collection.insert_one(fields2).unwrap();

    // Delete all inactive users
    let deleted = collection.delete_many(&json!({"active": false})).unwrap();
    assert_eq!(deleted, 2);

    // Now insert new documents with the same emails - should succeed
    let mut fields3 = std::collections::HashMap::new();
    fields3.insert("name".to_string(), json!("NewAlice"));
    fields3.insert("email".to_string(), json!("alice@example.com"));
    let result1 = collection.insert_one(fields3);
    assert!(result1.is_ok(), "Insert alice email after delete_many should succeed");

    let mut fields4 = std::collections::HashMap::new();
    fields4.insert("name".to_string(), json!("NewBob"));
    fields4.insert("email".to_string(), json!("bob@example.com"));
    let result2 = collection.insert_one(fields4);
    assert!(result2.is_ok(), "Insert bob email after delete_many should succeed");
}

#[test]
fn test_update_one_changes_indexed_value() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");

    let db = DatabaseCore::open(&db_path).unwrap();
    let collection = db.collection("users").unwrap();

    // Create unique index on email
    collection.create_index("email".to_string(), true).unwrap();

    // Insert document
    let mut fields = std::collections::HashMap::new();
    fields.insert("name".to_string(), json!("Alice"));
    fields.insert("email".to_string(), json!("alice@example.com"));
    collection.insert_one(fields).unwrap();

    // Update email to a new value
    let result = collection.update_one(
        &json!({"name": "Alice"}),
        &json!({"$set": {"email": "alice.new@example.com"}}),
    );
    assert!(result.is_ok());

    // The old email should now be available for reuse
    let mut fields2 = std::collections::HashMap::new();
    fields2.insert("name".to_string(), json!("Bob"));
    fields2.insert("email".to_string(), json!("alice@example.com"));
    let result2 = collection.insert_one(fields2);

    assert!(
        result2.is_ok(),
        "Old email should be available after update, got: {:?}",
        result2.err()
    );
}
