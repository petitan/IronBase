// Integration tests for MongoLite Core
use ironbase_core::{DatabaseCore, Document, DocumentId, StorageEngine};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use tempfile::TempDir;

// Helper to create test storage
fn create_test_storage() -> (TempDir, StorageEngine) {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");
    let storage = StorageEngine::open(&db_path).unwrap();
    (temp_dir, storage)
}

// Helper to create test document
fn create_doc(id: i64, name: &str, age: i64) -> Document {
    let mut fields = HashMap::new();
    fields.insert("name".to_string(), json!(name));
    fields.insert("age".to_string(), json!(age));
    Document::new(DocumentId::Int(id), fields)
}

#[test]
fn test_insert_and_read_document() {
    let (_temp, mut storage) = create_test_storage();
    storage.create_collection("users").unwrap();

    // Insert document
    let doc = create_doc(1, "Alice", 30);
    let doc_json = doc.to_json().unwrap();
    let offset = storage.write_data(doc_json.as_bytes()).unwrap();

    // Read back
    let data = storage.read_data(offset).unwrap();
    let json_str = String::from_utf8(data).unwrap();
    let restored = Document::from_json(&json_str).unwrap();

    assert_eq!(restored.id, DocumentId::Int(1));
    assert_eq!(restored.get("name").unwrap(), &json!("Alice"));
    assert_eq!(restored.get("age").unwrap(), &json!(30));
}

#[test]
fn test_multiple_documents_in_collection() {
    let (_temp, mut storage) = create_test_storage();
    storage.create_collection("users").unwrap();

    // Insert multiple documents
    let docs = vec![
        create_doc(1, "Alice", 30),
        create_doc(2, "Bob", 25),
        create_doc(3, "Carol", 35),
    ];

    let mut offsets = Vec::new();
    for doc in docs {
        let doc_json = doc.to_json().unwrap();
        let offset = storage.write_data(doc_json.as_bytes()).unwrap();
        offsets.push(offset);
    }

    // Read all back
    assert_eq!(offsets.len(), 3);

    for (i, offset) in offsets.iter().enumerate() {
        let data = storage.read_data(*offset).unwrap();
        let json_str = String::from_utf8(data).unwrap();
        let doc = Document::from_json(&json_str).unwrap();
        assert_eq!(doc.id, DocumentId::Int((i + 1) as i64));
    }
}

#[test]
fn test_collection_isolation() {
    let (_temp, mut storage) = create_test_storage();
    storage.create_collection("users").unwrap();
    storage.create_collection("posts").unwrap();

    // Get metadata
    let users_meta = storage.get_collection_meta("users").unwrap();
    let posts_meta = storage.get_collection_meta("posts").unwrap();

    // Both should have different metadata but same data_offset (since both empty)
    assert_eq!(users_meta.name, "users");
    assert_eq!(posts_meta.name, "posts");
    assert_eq!(users_meta.document_count, 0);
    assert_eq!(posts_meta.document_count, 0);
}

#[test]
fn test_document_count_tracking() {
    let (_temp, mut storage) = create_test_storage();
    storage.create_collection("users").unwrap();

    // Update document count manually (simulating collection operations)
    {
        let meta = storage.get_collection_meta_mut("users").unwrap();
        meta.document_count = 5;
    }

    // Verify
    let meta = storage.get_collection_meta("users").unwrap();
    assert_eq!(meta.document_count, 5);
}

#[test]
fn test_last_id_increment() {
    let (_temp, mut storage) = create_test_storage();
    storage.create_collection("users").unwrap();

    // Simulate ID generation
    let id1 = {
        let meta = storage.get_collection_meta_mut("users").unwrap();
        let id = DocumentId::new_auto(meta.last_id);
        meta.last_id += 1;
        id
    };

    let id2 = {
        let meta = storage.get_collection_meta_mut("users").unwrap();
        let id = DocumentId::new_auto(meta.last_id);
        meta.last_id += 1;
        id
    };

    assert_eq!(id1, DocumentId::Int(1));
    assert_eq!(id2, DocumentId::Int(2));
}

#[test]
fn test_persistence_across_reopens() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");

    // First session - create and write
    {
        let mut storage = StorageEngine::open(&db_path).unwrap();
        storage.create_collection("users").unwrap();

        let doc = create_doc(1, "Alice", 30);
        let doc_json = doc.to_json().unwrap();
        storage.write_data(doc_json.as_bytes()).unwrap();

        // Update metadata
        {
            let meta = storage.get_collection_meta_mut("users").unwrap();
            meta.document_count = 1;
            meta.last_id = 1;
        }

        storage.flush().unwrap();
    }

    // Second session - reopen and verify
    {
        let storage = StorageEngine::open(&db_path).unwrap();
        let meta = storage.get_collection_meta("users").unwrap();

        assert_eq!(meta.name, "users");
        assert_eq!(meta.document_count, 1);
        assert_eq!(meta.last_id, 1);
    }
}

#[test]
fn test_document_with_collection_field() {
    let (_temp, mut storage) = create_test_storage();
    storage.create_collection("users").unwrap();

    // Create document with _collection field (for isolation)
    let mut doc = create_doc(1, "Alice", 30);
    doc.set("_collection".to_string(), json!("users"));

    let doc_json = doc.to_json().unwrap();
    let offset = storage.write_data(doc_json.as_bytes()).unwrap();

    // Read back and verify _collection field
    let data = storage.read_data(offset).unwrap();
    let json_str = String::from_utf8(data).unwrap();
    let restored = Document::from_json(&json_str).unwrap();

    assert_eq!(restored.get("_collection").unwrap(), &json!("users"));
}

#[test]
fn test_collection_distinct_values() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("distinct.mlite");
    let db = DatabaseCore::<StorageEngine>::open(&db_path).unwrap();
    let coll = db.collection("products").unwrap();

    // Insert documents through CollectionCore to ensure catalog is populated
    coll.insert_one(HashMap::from([
        ("name".to_string(), json!("Laptop")),
        ("category".to_string(), json!("electronics")),
    ]))
    .unwrap();

    coll.insert_one(HashMap::from([
        ("name".to_string(), json!("Desk")),
        ("category".to_string(), json!("furniture")),
    ]))
    .unwrap();

    let distinct = coll
        .distinct("category", &Value::Object(Default::default()))
        .unwrap();
    assert_eq!(distinct.len(), 2);
    let categories: HashSet<_> = distinct.into_iter().collect();
    assert!(categories.contains(&json!("electronics")));
    assert!(categories.contains(&json!("furniture")));
}

#[test]
fn test_tombstone_pattern() {
    let (_temp, mut storage) = create_test_storage();
    storage.create_collection("users").unwrap();

    // Create document
    let doc = create_doc(1, "Alice", 30);
    let doc_json = doc.to_json().unwrap();
    storage.write_data(doc_json.as_bytes()).unwrap();

    // Create tombstone version
    let mut tombstone = create_doc(1, "Alice", 30);
    tombstone.set("_tombstone".to_string(), json!(true));

    let tombstone_json = tombstone.to_json().unwrap();
    let tombstone_offset = storage.write_data(tombstone_json.as_bytes()).unwrap();

    // Read tombstone
    let data = storage.read_data(tombstone_offset).unwrap();
    let json_str = String::from_utf8(data).unwrap();
    let restored = Document::from_json(&json_str).unwrap();

    assert_eq!(restored.get("_tombstone").unwrap(), &json!(true));
}

#[test]
fn test_update_pattern() {
    let (_temp, mut storage) = create_test_storage();
    storage.create_collection("users").unwrap();

    // Original document
    let doc = create_doc(1, "Alice", 30);
    let doc_json = doc.to_json().unwrap();
    storage.write_data(doc_json.as_bytes()).unwrap();

    // Updated version
    let mut updated = create_doc(1, "Alice", 31); // Age changed
    updated.set("updated".to_string(), json!(true));

    let updated_json = updated.to_json().unwrap();
    let updated_offset = storage.write_data(updated_json.as_bytes()).unwrap();

    // Read updated version
    let data = storage.read_data(updated_offset).unwrap();
    let json_str = String::from_utf8(data).unwrap();
    let restored = Document::from_json(&json_str).unwrap();

    assert_eq!(restored.get("age").unwrap(), &json!(31));
    assert_eq!(restored.get("updated").unwrap(), &json!(true));
}

#[test]
fn test_large_number_of_documents() {
    let (_temp, mut storage) = create_test_storage();
    storage.create_collection("users").unwrap();

    // Insert 100 documents
    let mut offsets = Vec::new();
    for i in 0..100 {
        let doc = create_doc(i, &format!("User{}", i), 20 + (i % 50));
        let doc_json = doc.to_json().unwrap();
        let offset = storage.write_data(doc_json.as_bytes()).unwrap();
        offsets.push(offset);
    }

    assert_eq!(offsets.len(), 100);

    // Verify first and last
    let first_data = storage.read_data(offsets[0]).unwrap();
    let first_doc = Document::from_json(&String::from_utf8(first_data).unwrap()).unwrap();
    assert_eq!(first_doc.id, DocumentId::Int(0));

    let last_data = storage.read_data(offsets[99]).unwrap();
    let last_doc = Document::from_json(&String::from_utf8(last_data).unwrap()).unwrap();
    assert_eq!(last_doc.id, DocumentId::Int(99));
}

#[test]
fn test_stats_with_collections() {
    let (_temp, mut storage) = create_test_storage();
    storage.create_collection("users").unwrap();
    storage.create_collection("posts").unwrap();

    let stats = storage.stats();

    assert_eq!(stats["collection_count"], 2);

    let collections = stats["collections"].as_array().unwrap();
    assert_eq!(collections.len(), 2);

    // Check collection names
    let names: Vec<String> = collections
        .iter()
        .map(|c| c["name"].as_str().unwrap().to_string())
        .collect();

    assert!(names.contains(&"users".to_string()));
    assert!(names.contains(&"posts".to_string()));
}

#[test]
fn test_schema_validation_blocks_invalid_insert() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("schema_insert.mlite");
    let db = DatabaseCore::<StorageEngine>::open(&db_path).unwrap();
    let collection = db.collection("users").unwrap();

    collection
        .set_schema(Some(json!({
            "type": "object",
            "required": ["name", "age"],
            "properties": {
                "name": {"type": "string"},
                "age": {"type": "number"}
            }
        })))
        .unwrap();

    let mut invalid = HashMap::new();
    invalid.insert("name".to_string(), json!("Alice"));
    assert!(collection.insert_one(invalid).is_err());

    let mut valid = HashMap::new();
    valid.insert("name".to_string(), json!("Bob"));
    valid.insert("age".to_string(), json!(30));
    collection.insert_one(valid).unwrap();
}

#[test]
fn test_schema_validation_blocks_invalid_update() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("schema_update.mlite");
    let db = DatabaseCore::<StorageEngine>::open(&db_path).unwrap();
    let collection = db.collection("users").unwrap();

    collection
        .set_schema(Some(json!({
            "type": "object",
            "required": ["name", "age"],
            "properties": {
                "name": {"type": "string"},
                "age": {"type": "number"}
            }
        })))
        .unwrap();

    let mut doc = HashMap::new();
    doc.insert("name".to_string(), json!("Carol"));
    doc.insert("age".to_string(), json!(28));
    collection.insert_one(doc).unwrap();

    let result =
        collection.update_one(&json!({"name": "Carol"}), &json!({"$unset": {"age": true}}));
    assert!(result.is_err());
}

#[test]
fn test_nested_field_queries_via_collection_core() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("nested_query.mlite");
    let db = DatabaseCore::<StorageEngine>::open(&db_path).unwrap();
    let coll = db.collection("customers").unwrap();

    coll.insert_one(HashMap::from([
        ("name".to_string(), json!("Anna")),
        (
            "address".to_string(),
            json!({"city": "Budapest", "zip": 1111}),
        ),
        ("stats".to_string(), json!({"login_count": 42})),
    ]))
    .unwrap();

    coll.insert_one(HashMap::from([
        ("name".to_string(), json!("Bence")),
        (
            "address".to_string(),
            json!({"city": "Debrecen", "zip": 4025}),
        ),
        ("stats".to_string(), json!({"login_count": 5})),
    ]))
    .unwrap();

    let found = coll.find_one(&json!({"address.city": "Budapest"})).unwrap();
    assert!(found.is_some());
    assert_eq!(
        found.unwrap().get("name").and_then(|v| v.as_str()),
        Some("Anna")
    );

    let matched = coll
        .find(&json!({"stats.login_count": {"$gte": 40}}))
        .unwrap();
    assert_eq!(matched.len(), 1);
    assert_eq!(
        matched[0].get("name").and_then(|v| v.as_str()),
        Some("Anna")
    );
}

// =============================================================================
// NESTED DOCUMENT INTEGRATION TESTS - End-to-end scenarios with persistence
// =============================================================================

#[test]
fn test_nested_crud_workflow_with_persistence() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("nested_crud.mlite");

    // Session 1: Create and insert nested documents
    {
        let db = DatabaseCore::<StorageEngine>::open(&db_path).unwrap();
        let coll = db.collection("companies").unwrap();

        coll.insert_one(HashMap::from([
            ("name".to_string(), json!("TechCorp")),
            ("location".to_string(), json!({
                "country": "USA",
                "city": "San Francisco",
                "address": {
                    "street": "123 Tech Blvd",
                    "zip": "94105"
                }
            })),
            ("stats".to_string(), json!({
                "employees": 500,
                "revenue": 50000000,
                "rating": 4.8
            }))
        ])).unwrap();

        coll.insert_one(HashMap::from([
            ("name".to_string(), json!("DataSoft")),
            ("location".to_string(), json!({
                "country": "USA",
                "city": "New York",
                "address": {
                    "street": "456 Data Ave",
                    "zip": "10001"
                }
            })),
            ("stats".to_string(), json!({
                "employees": 200,
                "revenue": 20000000,
                "rating": 4.2
            }))
        ])).unwrap();
    }

    // Session 2: Reopen and verify, then update
    {
        let db = DatabaseCore::<StorageEngine>::open(&db_path).unwrap();
        let coll = db.collection("companies").unwrap();

        // Verify data persisted
        let results = coll.find(&json!({"location.country": "USA"})).unwrap();
        assert_eq!(results.len(), 2, "Should find 2 USA companies");

        // Query nested field
        let sf_company = coll.find_one(&json!({"location.city": "San Francisco"})).unwrap();
        assert!(sf_company.is_some());
        assert_eq!(sf_company.unwrap()["name"], "TechCorp");

        // Update nested field
        coll.update_one(
            &json!({"name": "TechCorp"}),
            &json!({"$set": {"stats.employees": 550}})
        ).unwrap();

        // Update deep nested field
        coll.update_one(
            &json!({"name": "DataSoft"}),
            &json!({"$set": {"location.address.zip": "10002"}})
        ).unwrap();
    }

    // Session 3: Verify updates persisted
    {
        let db = DatabaseCore::<StorageEngine>::open(&db_path).unwrap();
        let coll = db.collection("companies").unwrap();

        let tech = coll.find_one(&json!({"name": "TechCorp"})).unwrap().unwrap();
        assert_eq!(tech["stats"]["employees"], 550);

        let data = coll.find_one(&json!({"name": "DataSoft"})).unwrap().unwrap();
        assert_eq!(data["location"]["address"]["zip"], "10002");
    }
}

#[test]
fn test_nested_aggregation_with_persistence() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("nested_agg.mlite");

    // Session 1: Insert data
    {
        let db = DatabaseCore::<StorageEngine>::open(&db_path).unwrap();
        let coll = db.collection("sales").unwrap();

        let regions = vec![
            ("North", "Electronics", 1000),
            ("North", "Clothing", 500),
            ("South", "Electronics", 800),
            ("South", "Clothing", 600),
            ("North", "Electronics", 1200),
        ];

        for (region, category, amount) in regions {
            coll.insert_one(HashMap::from([
                ("region".to_string(), json!({
                    "name": region,
                    "active": true
                })),
                ("product".to_string(), json!({
                    "category": category,
                    "details": {
                        "amount": amount
                    }
                }))
            ])).unwrap();
        }
    }

    // Session 2: Run aggregation after reopen
    {
        let db = DatabaseCore::<StorageEngine>::open(&db_path).unwrap();
        let coll = db.collection("sales").unwrap();

        // Aggregate by nested region.name
        let results = coll.aggregate(&json!([
            {"$group": {
                "_id": "$region.name",
                "totalAmount": {"$sum": "$product.details.amount"},
                "count": {"$sum": 1}
            }},
            {"$sort": {"totalAmount": -1}}
        ])).unwrap();

        assert_eq!(results.len(), 2);

        // North: 1000 + 500 + 1200 = 2700
        let north = results.iter().find(|r| r["_id"] == "North").unwrap();
        assert_eq!(north["totalAmount"], 2700);
        assert_eq!(north["count"], 3);

        // South: 800 + 600 = 1400
        let south = results.iter().find(|r| r["_id"] == "South").unwrap();
        assert_eq!(south["totalAmount"], 1400);
        assert_eq!(south["count"], 2);
    }
}

#[test]
fn test_nested_index_with_persistence() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("nested_index.mlite");

    // Session 1: Create index and insert data
    {
        let db = DatabaseCore::<StorageEngine>::open(&db_path).unwrap();
        let coll = db.collection("users").unwrap();

        // Create index on nested field
        coll.create_index("profile.score".to_string(), false).unwrap();

        for i in 0..100 {
            coll.insert_one(HashMap::from([
                ("name".to_string(), json!(format!("User{}", i))),
                ("profile".to_string(), json!({
                    "score": i * 10,
                    "level": if i < 50 { "junior" } else { "senior" }
                }))
            ])).unwrap();
        }
    }

    // Session 2: Verify index exists and works after reopen
    {
        let db = DatabaseCore::<StorageEngine>::open(&db_path).unwrap();
        let coll = db.collection("users").unwrap();

        // List indexes - returns Vec<String> of index names
        let indexes = coll.list_indexes();
        assert!(
            indexes.iter().any(|idx| idx.contains("profile.score")),
            "Index on profile.score should exist after reopen, got: {:?}",
            indexes
        );

        // Verify query works and returns correct results
        let results = coll.find(&json!({"profile.score": {"$gte": 900}})).unwrap();
        assert_eq!(results.len(), 10, "Should find users with score >= 900");

        // Verify query explain works
        let explain = coll.explain(&json!({"profile.score": {"$gte": 500}})).unwrap();
        assert!(!explain.is_null(), "Explain should return valid result");
    }
}

#[test]
fn test_nested_delete_with_persistence() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("nested_delete.mlite");

    // Session 1: Insert data
    {
        let db = DatabaseCore::<StorageEngine>::open(&db_path).unwrap();
        let coll = db.collection("orders").unwrap();

        for i in 0..10 {
            coll.insert_one(HashMap::from([
                ("order_id".to_string(), json!(i)),
                ("customer".to_string(), json!({
                    "name": format!("Customer{}", i % 3),
                    "tier": if i < 5 { "bronze" } else { "gold" }
                })),
                ("amount".to_string(), json!(100 * (i + 1)))
            ])).unwrap();
        }
    }

    // Session 2: Delete by nested field and verify
    {
        let db = DatabaseCore::<StorageEngine>::open(&db_path).unwrap();
        let coll = db.collection("orders").unwrap();

        // Count before delete
        let before = coll.count_documents(&json!({})).unwrap();
        assert_eq!(before, 10);

        // Delete bronze tier orders
        let deleted = coll.delete_many(&json!({"customer.tier": "bronze"})).unwrap();
        assert_eq!(deleted, 5);

        // Verify remaining
        let remaining = coll.find(&json!({})).unwrap();
        assert_eq!(remaining.len(), 5);

        // All remaining should be gold tier
        for doc in remaining {
            assert_eq!(doc["customer"]["tier"], "gold");
        }
    }

    // Session 3: Verify delete persisted
    {
        let db = DatabaseCore::<StorageEngine>::open(&db_path).unwrap();
        let coll = db.collection("orders").unwrap();

        let count = coll.count_documents(&json!({})).unwrap();
        assert_eq!(count, 5, "Delete should persist across sessions");

        let bronze = coll.find(&json!({"customer.tier": "bronze"})).unwrap();
        assert_eq!(bronze.len(), 0, "No bronze tier orders should remain");
    }
}

#[test]
fn test_nested_find_options_with_persistence() {
    use ironbase_core::FindOptions;

    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("nested_options.mlite");

    // Session 1: Insert data
    {
        let db = DatabaseCore::<StorageEngine>::open(&db_path).unwrap();
        let coll = db.collection("products").unwrap();

        let products = vec![
            ("Laptop", "Electronics", 999.99),
            ("Phone", "Electronics", 599.99),
            ("Desk", "Furniture", 299.99),
            ("Chair", "Furniture", 149.99),
            ("Tablet", "Electronics", 449.99),
        ];

        for (name, category, price) in products {
            coll.insert_one(HashMap::from([
                ("name".to_string(), json!(name)),
                ("info".to_string(), json!({
                    "category": category,
                    "pricing": {
                        "price": price,
                        "currency": "USD"
                    }
                }))
            ])).unwrap();
        }
    }

    // Session 2: Query with options after reopen
    {
        let db = DatabaseCore::<StorageEngine>::open(&db_path).unwrap();
        let coll = db.collection("products").unwrap();

        // Sort by nested price descending, limit 3
        let options = FindOptions::new()
            .with_sort(vec![("info.pricing.price".to_string(), -1)])
            .with_limit(3);

        let results = coll.find_with_options(&json!({}), options).unwrap();

        assert_eq!(results.len(), 3);
        // Should be sorted by price: Laptop(999.99), Phone(599.99), Tablet(449.99)
        assert_eq!(results[0]["name"], "Laptop");
        assert_eq!(results[1]["name"], "Phone");
        assert_eq!(results[2]["name"], "Tablet");

        // Query only electronics, sorted by price ascending
        let options = FindOptions::new()
            .with_sort(vec![("info.pricing.price".to_string(), 1)]);

        let electronics = coll.find_with_options(
            &json!({"info.category": "Electronics"}),
            options
        ).unwrap();

        assert_eq!(electronics.len(), 3);
        assert_eq!(electronics[0]["name"], "Tablet");  // 449.99
        assert_eq!(electronics[1]["name"], "Phone");   // 599.99
        assert_eq!(electronics[2]["name"], "Laptop");  // 999.99
    }
}

#[test]
fn test_deeply_nested_update_create_path() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("deep_update.mlite");

    // Session 1: Insert simple doc and update to create deep nested path
    {
        let db = DatabaseCore::<StorageEngine>::open(&db_path).unwrap();
        let coll = db.collection("config").unwrap();

        coll.insert_one(HashMap::from([
            ("name".to_string(), json!("app_config")),
            ("version".to_string(), json!(1))
        ])).unwrap();

        // Create deep nested path via update
        coll.update_one(
            &json!({"name": "app_config"}),
            &json!({"$set": {
                "settings.database.connection.timeout": 30,
                "settings.database.connection.retries": 3,
                "settings.cache.enabled": true
            }})
        ).unwrap();
    }

    // Session 2: Verify deep nested structure persisted
    {
        let db = DatabaseCore::<StorageEngine>::open(&db_path).unwrap();
        let coll = db.collection("config").unwrap();

        let config = coll.find_one(&json!({"name": "app_config"})).unwrap().unwrap();

        assert_eq!(config["settings"]["database"]["connection"]["timeout"], 30);
        assert_eq!(config["settings"]["database"]["connection"]["retries"], 3);
        assert_eq!(config["settings"]["cache"]["enabled"], true);

        // Query by deep nested field
        let found = coll.find(&json!({
            "settings.database.connection.timeout": {"$gte": 20}
        })).unwrap();
        assert_eq!(found.len(), 1);
    }
}

#[test]
fn test_mixed_nested_and_flat_fields() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("mixed_fields.mlite");

    {
        let db = DatabaseCore::<StorageEngine>::open(&db_path).unwrap();
        let coll = db.collection("events").unwrap();

        coll.insert_one(HashMap::from([
            ("event_type".to_string(), json!("purchase")),
            ("timestamp".to_string(), json!("2024-01-15T10:30:00Z")),
            ("user".to_string(), json!({
                "id": 123,
                "profile": {
                    "name": "Alice",
                    "premium": true
                }
            })),
            ("data".to_string(), json!({
                "product_id": "P001",
                "amount": 99.99
            }))
        ])).unwrap();

        coll.insert_one(HashMap::from([
            ("event_type".to_string(), json!("view")),
            ("timestamp".to_string(), json!("2024-01-15T11:00:00Z")),
            ("user".to_string(), json!({
                "id": 456,
                "profile": {
                    "name": "Bob",
                    "premium": false
                }
            })),
            ("data".to_string(), json!({
                "page": "/products"
            }))
        ])).unwrap();
    }

    {
        let db = DatabaseCore::<StorageEngine>::open(&db_path).unwrap();
        let coll = db.collection("events").unwrap();

        // Query combining flat and nested conditions
        let results = coll.find(&json!({
            "event_type": "purchase",
            "user.profile.premium": true
        })).unwrap();

        assert_eq!(results.len(), 1);
        assert_eq!(results[0]["user"]["profile"]["name"], "Alice");

        // Aggregation mixing flat and nested
        let agg = coll.aggregate(&json!([
            {"$group": {
                "_id": "$user.profile.premium",
                "event_count": {"$sum": 1}
            }}
        ])).unwrap();

        assert_eq!(agg.len(), 2);
    }
}
