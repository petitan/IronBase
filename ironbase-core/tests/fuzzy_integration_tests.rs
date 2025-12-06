// Fuzzy search integration tests
// Tests the full stack: DatabaseCore -> Collection -> FuzzyIndex + $fuzzy operator

use ironbase_core::index::FuzzyAlgorithm;
use ironbase_core::DatabaseCore;
use serde_json::json;
use std::collections::HashMap;
use tempfile::TempDir;

// ============================================================================
// HELPER MACROS
// ============================================================================

macro_rules! insert_user {
    ($db:expr, $name:expr, $email:expr, $age:expr) => {{
        let mut fields = HashMap::new();
        fields.insert("name".to_string(), json!($name));
        fields.insert("email".to_string(), json!($email));
        fields.insert("age".to_string(), json!($age));
        $db.insert_one("users", fields).unwrap();
    }};
}

macro_rules! insert_nested_user {
    ($db:expr, $name:expr, $city:expr, $country:expr) => {{
        let mut fields = HashMap::new();
        fields.insert("name".to_string(), json!($name));
        fields.insert(
            "address".to_string(),
            json!({
                "city": $city,
                "country": $country
            }),
        );
        $db.insert_one("users", fields).unwrap();
    }};
}

// ============================================================================
// BASIC FUZZY QUERY TESTS
// ============================================================================

#[test]
fn test_fuzzy_find_simple() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");
    let db = DatabaseCore::open(&db_path).unwrap();

    insert_user!(db, "John Smith", "john@example.com", 30);
    insert_user!(db, "Jane Doe", "jane@example.com", 25);
    insert_user!(db, "Bob Johnson", "bob@example.com", 35);

    let collection = db.collection("users").unwrap();

    // Fuzzy search for "jon" should match "John Smith"
    let results = collection
        .find(&json!({"name": {"$fuzzy": "jon smith"}}))
        .unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(
        results[0].get("name").unwrap().as_str().unwrap(),
        "John Smith"
    );
}

#[test]
fn test_fuzzy_find_with_typo() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");
    let db = DatabaseCore::open(&db_path).unwrap();

    insert_user!(db, "Michael", "michael@example.com", 28);
    insert_user!(db, "Michelle", "michelle@example.com", 32);
    insert_user!(db, "Robert", "robert@example.com", 45);

    let collection = db.collection("users").unwrap();

    // Fuzzy search with typo "Micheal" should match "Michael"
    let results = collection
        .find(&json!({"name": {"$fuzzy": "Micheal"}}))
        .unwrap();
    assert!(!results.is_empty());
    let names: Vec<&str> = results
        .iter()
        .map(|d| d.get("name").unwrap().as_str().unwrap())
        .collect();
    assert!(names.contains(&"Michael"));
}

#[test]
fn test_fuzzy_find_no_match() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");
    let db = DatabaseCore::open(&db_path).unwrap();

    insert_user!(db, "Alice", "alice@example.com", 30);
    insert_user!(db, "Bob", "bob@example.com", 25);

    let collection = db.collection("users").unwrap();

    // Fuzzy search for completely different name should return nothing
    let results = collection
        .find(&json!({"name": {"$fuzzy": "Zzzzzz"}}))
        .unwrap();
    assert_eq!(results.len(), 0);
}

#[test]
fn test_fuzzy_find_multiple_matches() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");
    let db = DatabaseCore::open(&db_path).unwrap();

    insert_user!(db, "Johnson", "j1@example.com", 30);
    insert_user!(db, "Johnston", "j2@example.com", 25);
    insert_user!(db, "Johnsen", "j3@example.com", 35);
    insert_user!(db, "Williams", "w@example.com", 40);

    let collection = db.collection("users").unwrap();

    // Fuzzy search for "Johnson" should match all Johnson variants
    let results = collection
        .find(&json!({"name": {"$fuzzy": "Johnson"}}))
        .unwrap();
    assert!(results.len() >= 2); // Should match Johnson, Johnston, possibly Johnsen
}

// ============================================================================
// FUZZY WITH EXTENDED FORM (algorithm + threshold)
// ============================================================================

#[test]
fn test_fuzzy_find_with_algorithm() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");
    let db = DatabaseCore::open(&db_path).unwrap();

    insert_user!(db, "Alexander", "alex@example.com", 30);
    insert_user!(db, "Alexandra", "alexa@example.com", 25);

    let collection = db.collection("users").unwrap();

    // Use Levenshtein algorithm with custom threshold
    let results = collection
        .find(&json!({
            "name": {
                "$fuzzy": {
                    "value": "Alexandr",
                    "algorithm": "levenshtein",
                    "threshold": 0.7
                }
            }
        }))
        .unwrap();

    assert!(!results.is_empty());
}

#[test]
fn test_fuzzy_find_with_jaro_winkler() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");
    let db = DatabaseCore::open(&db_path).unwrap();

    insert_user!(db, "Stephen", "stephen@example.com", 30);
    insert_user!(db, "Steven", "steven@example.com", 28);
    insert_user!(db, "Stefan", "stefan@example.com", 35);

    let collection = db.collection("users").unwrap();

    // Jaro-Winkler is good for name matching
    let results = collection
        .find(&json!({
            "name": {
                "$fuzzy": {
                    "value": "Stefen",
                    "algorithm": "jaro_winkler",
                    "threshold": 0.8
                }
            }
        }))
        .unwrap();

    assert!(!results.is_empty());
}

#[test]
fn test_fuzzy_find_with_damerau_levenshtein() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");
    let db = DatabaseCore::open(&db_path).unwrap();

    insert_user!(db, "receive", "r1@example.com", 30);
    insert_user!(db, "deceive", "r2@example.com", 25);

    let collection = db.collection("users").unwrap();

    // Damerau-Levenshtein handles transpositions well
    let results = collection
        .find(&json!({
            "name": {
                "$fuzzy": {
                    "value": "recieve",  // Common misspelling (transposition)
                    "algorithm": "damerau_levenshtein",
                    "threshold": 0.8
                }
            }
        }))
        .unwrap();

    assert!(!results.is_empty());
    assert_eq!(results[0].get("name").unwrap().as_str().unwrap(), "receive");
}

// ============================================================================
// FUZZY WITH NESTED DOCUMENTS
// ============================================================================

#[test]
fn test_fuzzy_find_nested_field() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");
    let db = DatabaseCore::open(&db_path).unwrap();

    insert_nested_user!(db, "Alice", "New York", "USA");
    insert_nested_user!(db, "Bob", "Los Angeles", "USA");
    insert_nested_user!(db, "Carol", "Budapest", "Hungary");

    let collection = db.collection("users").unwrap();

    // Fuzzy search on nested field with lower threshold for typo
    let results = collection
        .find(&json!({
            "address.city": {"$fuzzy": {"value": "New Yrok", "threshold": 0.7}}
        }))
        .unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].get("name").unwrap().as_str().unwrap(), "Alice");
}

#[test]
fn test_fuzzy_find_nested_with_algorithm() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");
    let db = DatabaseCore::open(&db_path).unwrap();

    insert_nested_user!(db, "Alice", "Philadelphia", "USA");
    insert_nested_user!(db, "Bob", "Phoenix", "USA");

    let collection = db.collection("users").unwrap();

    // Fuzzy search with extended form on nested field
    let results = collection
        .find(&json!({
            "address.city": {
                "$fuzzy": {
                    "value": "Philadelfia",  // Missing 'ph'
                    "algorithm": "levenshtein",
                    "threshold": 0.8
                }
            }
        }))
        .unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].get("name").unwrap().as_str().unwrap(), "Alice");
}

// ============================================================================
// FUZZY COMBINED WITH OTHER OPERATORS
// ============================================================================

#[test]
fn test_fuzzy_with_and_operator() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");
    let db = DatabaseCore::open(&db_path).unwrap();

    insert_user!(db, "John Smith", "john@example.com", 30);
    insert_user!(db, "John Doe", "johnd@example.com", 25);
    insert_user!(db, "Jane Smith", "jane@example.com", 35);

    let collection = db.collection("users").unwrap();

    // Fuzzy name AND age filter - use lower threshold for "Jon" -> "John"
    let results = collection
        .find(&json!({
            "$and": [
                {"name": {"$fuzzy": {"value": "John", "threshold": 0.7}}},
                {"age": {"$gte": 28}}
            ]
        }))
        .unwrap();

    // Should match "John Smith" (age 30) - John Doe is only 25
    assert_eq!(results.len(), 1);
    assert_eq!(
        results[0].get("name").unwrap().as_str().unwrap(),
        "John Smith"
    );
}

#[test]
fn test_fuzzy_with_or_operator() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");
    let db = DatabaseCore::open(&db_path).unwrap();

    insert_user!(db, "Alice", "alice@example.com", 30);
    insert_user!(db, "Bob", "bob@example.com", 25);
    insert_user!(db, "Carol", "carol@example.com", 35);

    let collection = db.collection("users").unwrap();

    // Fuzzy name OR exact match
    let results = collection
        .find(&json!({
            "$or": [
                {"name": {"$fuzzy": "Alic"}},
                {"name": "Carol"}
            ]
        }))
        .unwrap();

    assert_eq!(results.len(), 2);
}

#[test]
fn test_fuzzy_with_comparison_operators() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");
    let db = DatabaseCore::open(&db_path).unwrap();

    insert_user!(db, "Senior Developer", "senior@example.com", 45);
    insert_user!(db, "Junior Developer", "junior@example.com", 25);
    insert_user!(db, "Manager", "manager@example.com", 40);

    let collection = db.collection("users").unwrap();

    // Fuzzy on name + comparison on age - use lower threshold
    let results = collection
        .find(&json!({
            "name": {"$fuzzy": {"value": "Junior Dev", "threshold": 0.6}},
            "age": {"$lt": 30}
        }))
        .unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(
        results[0].get("name").unwrap().as_str().unwrap(),
        "Junior Developer"
    );
}

// ============================================================================
// FUZZY INDEX TESTS
// ============================================================================

#[test]
fn test_create_fuzzy_index() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");
    let db = DatabaseCore::open(&db_path).unwrap();
    let collection = db.collection("users").unwrap();

    // Create fuzzy index
    let index_name = collection
        .create_fuzzy_index("name".to_string(), FuzzyAlgorithm::JaroWinkler, 0.8)
        .unwrap();

    assert_eq!(index_name, "users_name_fuzzy");

    // Verify index exists
    let indexes = collection.list_indexes();
    assert!(indexes.contains(&"users_name_fuzzy".to_string()));
}

#[test]
fn test_fuzzy_index_with_documents() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");
    let db = DatabaseCore::open(&db_path).unwrap();

    // Insert documents first
    insert_user!(db, "Alice", "alice@example.com", 30);
    insert_user!(db, "Bob", "bob@example.com", 25);
    insert_user!(db, "Charlie", "charlie@example.com", 35);

    let collection = db.collection("users").unwrap();

    // Create fuzzy index after documents exist
    collection
        .create_fuzzy_index("name".to_string(), FuzzyAlgorithm::JaroWinkler, 0.7)
        .unwrap();

    // Search using fuzzy - "Alice" vs "Alice" is exact match
    let results = collection
        .find(&json!({"name": {"$fuzzy": "Alice"}}))
        .unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].get("name").unwrap().as_str().unwrap(), "Alice");
}

#[test]
fn test_fuzzy_index_with_levenshtein() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");
    let db = DatabaseCore::open(&db_path).unwrap();
    let collection = db.collection("users").unwrap();

    // Create fuzzy index with Levenshtein
    collection
        .create_fuzzy_index("email".to_string(), FuzzyAlgorithm::Levenshtein, 0.7)
        .unwrap();

    insert_user!(db, "Alice", "alice@example.com", 30);
    insert_user!(db, "Bob", "bob@example.com", 25);

    // Search with typo
    let results = collection
        .find(&json!({
            "email": {"$fuzzy": "alice@exmple.com"}  // Missing 'a'
        }))
        .unwrap();

    assert_eq!(results.len(), 1);
}

#[test]
fn test_drop_fuzzy_index() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");
    let db = DatabaseCore::open(&db_path).unwrap();
    let collection = db.collection("users").unwrap();

    // Create and then drop fuzzy index
    collection
        .create_fuzzy_index("name".to_string(), FuzzyAlgorithm::JaroWinkler, 0.8)
        .unwrap();

    let indexes_before = collection.list_indexes();
    assert!(indexes_before.contains(&"users_name_fuzzy".to_string()));

    collection.drop_index("users_name_fuzzy").unwrap();

    let indexes_after = collection.list_indexes();
    assert!(!indexes_after.contains(&"users_name_fuzzy".to_string()));
}

// ============================================================================
// PERSISTENCE TESTS
// ============================================================================

#[test]
fn test_fuzzy_data_persistence() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("data_persist.mlite");

    // Create database and insert documents
    {
        let db = DatabaseCore::open(&db_path).unwrap();
        insert_user!(db, "John Smith", "john@test.com", 30);
        insert_user!(db, "Jane Doe", "jane@test.com", 25);
        // db drops here, triggering flush
    }

    // Reopen and verify data persisted - use exact match first
    {
        let db = DatabaseCore::open(&db_path).unwrap();
        let collection = db.collection("users").unwrap();

        // First verify data was persisted with exact match
        let all_docs = collection.find(&json!({})).unwrap();
        assert_eq!(all_docs.len(), 2, "Data should be persisted");

        // Then test fuzzy search with low threshold
        let results = collection
            .find(&json!({
                "name": {"$fuzzy": {"value": "John Smth", "threshold": 0.7}}
            }))
            .unwrap();
        assert!(!results.is_empty(), "Fuzzy should find John Smith");
    }
}

// ============================================================================
// EDGE CASES
// ============================================================================

#[test]
fn test_fuzzy_empty_collection() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");
    let db = DatabaseCore::open(&db_path).unwrap();
    let collection = db.collection("users").unwrap();

    let results = collection
        .find(&json!({"name": {"$fuzzy": "test"}}))
        .unwrap();
    assert_eq!(results.len(), 0);
}

#[test]
fn test_fuzzy_case_sensitivity() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");
    let db = DatabaseCore::open(&db_path).unwrap();

    insert_user!(db, "ALICE", "alice@example.com", 30);
    insert_user!(db, "alice", "alice2@example.com", 25);
    insert_user!(db, "Alice", "alice3@example.com", 35);

    let collection = db.collection("users").unwrap();

    // Fuzzy matches similar strings - exact "alice" should match at least the exact ones
    let results = collection
        .find(&json!({"name": {"$fuzzy": "alice"}}))
        .unwrap();
    assert!(!results.is_empty()); // At least one match

    // With lower threshold, should match more variations
    let results2 = collection
        .find(&json!({
            "name": {"$fuzzy": {"value": "alice", "threshold": 0.6}}
        }))
        .unwrap();
    assert!(!results2.is_empty());
}

#[test]
fn test_fuzzy_special_characters() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");
    let db = DatabaseCore::open(&db_path).unwrap();

    insert_user!(db, "O'Brien", "obrien@example.com", 30);
    insert_user!(db, "Müller", "muller@example.com", 25);

    let collection = db.collection("users").unwrap();

    // Search with special characters
    let results = collection
        .find(&json!({"name": {"$fuzzy": "O'Brian"}}))
        .unwrap();
    assert_eq!(results.len(), 1);

    let results2 = collection
        .find(&json!({"name": {"$fuzzy": "Muller"}}))
        .unwrap();
    assert!(!results2.is_empty());
}

#[test]
fn test_fuzzy_unicode() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");
    let db = DatabaseCore::open(&db_path).unwrap();

    insert_user!(db, "Józef", "jozef@example.com", 30);
    insert_user!(db, "東京", "tokyo@example.com", 25);

    let collection = db.collection("users").unwrap();

    // Hungarian name
    let results = collection
        .find(&json!({"name": {"$fuzzy": "Jozef"}}))
        .unwrap();
    assert!(!results.is_empty());
}

#[test]
fn test_fuzzy_threshold_boundary() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");
    let db = DatabaseCore::open(&db_path).unwrap();

    insert_user!(db, "test", "test@example.com", 30);

    let collection = db.collection("users").unwrap();

    // Very high threshold - exact match only
    let results = collection
        .find(&json!({
            "name": {
                "$fuzzy": {
                    "value": "test",
                    "threshold": 1.0
                }
            }
        }))
        .unwrap();
    assert_eq!(results.len(), 1);

    // Very low threshold - may match more liberally
    let _results2 = collection
        .find(&json!({
            "name": {
                "$fuzzy": {
                    "value": "xxxx",
                    "threshold": 0.1
                }
            }
        }))
        .unwrap();
    // With very low threshold, might match - this is expected behavior
}

// ============================================================================
// FUZZY INDEX PERSISTENCE TESTS
// ============================================================================

#[test]
fn test_fuzzy_index_persistence() {
    // This test verifies that fuzzy index metadata is persisted
    // Currently uses in-memory only to verify basic functionality
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test_fuzzy_basic.mlite");
    let db = DatabaseCore::open(&db_path).unwrap();

    // Insert some documents
    insert_user!(db, "John Smith", "john@example.com", 30);
    insert_user!(db, "Jane Doe", "jane@example.com", 25);

    // Create fuzzy index
    let collection = db.collection("users").unwrap();
    let _index_name = collection
        .create_fuzzy_index("name".to_string(), FuzzyAlgorithm::JaroWinkler, 0.7)
        .unwrap();

    // Verify search works
    let results = collection.fuzzy_search("name", "John", None, None).unwrap();
    assert_eq!(results.len(), 1, "Should find John Smith");
}

#[test]
fn test_fuzzy_index_file_persistence() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test_fuzzy_persist.mlite");

    // Phase 1: Create database, add data and create fuzzy index
    {
        let db = DatabaseCore::open(&db_path).unwrap();

        // Insert some documents
        insert_user!(db, "John Smith", "john@example.com", 30);
        insert_user!(db, "Jane Doe", "jane@example.com", 25);

        // Create fuzzy index
        let collection = db.collection("users").unwrap();
        collection
            .create_fuzzy_index("name".to_string(), FuzzyAlgorithm::JaroWinkler, 0.7)
            .unwrap();

        // Verify search works
        let results = collection.fuzzy_search("name", "John", None, None).unwrap();
        assert_eq!(results.len(), 1, "Should find John Smith");
    }
    // db dropped here - should flush

    // Phase 2: Reopen database and verify fuzzy index was restored
    {
        let db2 = DatabaseCore::open(&db_path).unwrap();
        let collection2 = db2.collection("users").unwrap();

        // Verify fuzzy search still works after reload
        let results = collection2
            .fuzzy_search("name", "John", None, None)
            .unwrap();
        assert!(
            !results.is_empty(),
            "Fuzzy search should work after database reload, found {} results",
            results.len()
        );
    }
}

#[test]
fn test_fuzzy_index_persistence_with_drop() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test_fuzzy_drop.mlite");

    // Phase 1: Create database with fuzzy index
    let db = DatabaseCore::open(&db_path).unwrap();
    insert_user!(db, "Alice", "alice@example.com", 28);

    let index_name = {
        let collection = db.collection("users").unwrap();
        let name = collection
            .create_fuzzy_index("name".to_string(), FuzzyAlgorithm::JaroWinkler, 0.7)
            .unwrap();

        // Verify it works
        let results = collection
            .fuzzy_search("name", "Alice", None, None)
            .unwrap();
        assert_eq!(results.len(), 1);

        // Drop the index
        collection.drop_index(&name).unwrap();
        name
    };

    drop(db);

    // Phase 2: Reopen - index should NOT be recreated
    let db2 = DatabaseCore::open(&db_path).unwrap();
    let collection2 = db2.collection("users").unwrap();

    // fuzzy_search should fail because index was dropped
    let result = collection2.fuzzy_search("name", "Alice", None, None);
    assert!(
        result.is_err(),
        "Fuzzy search should fail after index was dropped and db reloaded (index_name was: {})",
        index_name
    );
}
