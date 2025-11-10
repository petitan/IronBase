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

    // TODO: Add index-based query test when query optimizer is implemented
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
