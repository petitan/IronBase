// Property-based tests using proptest
use ironbase_core::{Document, DocumentId, Query, StorageEngine};
use proptest::prelude::*;
use serde_json::json;
use std::collections::HashMap;
use tempfile::TempDir;

// ========== PROPERTY 1: Document Serialization Roundtrip ==========

proptest! {
    #[test]
    fn prop_document_roundtrip_int_id(id in 0i64..1000000, name in "[a-z]{1,50}", age in 0i64..150) {
        let mut fields = HashMap::new();
        fields.insert("name".to_string(), json!(name));
        fields.insert("age".to_string(), json!(age));

        let doc = Document::new(DocumentId::Int(id), fields);

        // Serialize to JSON
        let json_str = doc.to_json().unwrap();

        // Deserialize back
        let restored = Document::from_json(&json_str).unwrap();

        // Invariant: restored == original
        assert_eq!(restored.id, doc.id);
        assert_eq!(restored.get("name"), doc.get("name"));
        assert_eq!(restored.get("age"), doc.get("age"));
    }
}

proptest! {
    #[test]
    fn prop_document_roundtrip_string_id(id in "[a-zA-Z0-9]{1,20}", value in any::<i64>()) {
        let mut fields = HashMap::new();
        fields.insert("value".to_string(), json!(value));

        let doc = Document::new(DocumentId::String(id.clone()), fields);

        let json_str = doc.to_json().unwrap();
        let restored = Document::from_json(&json_str).unwrap();

        assert_eq!(restored.id, DocumentId::String(id));
        assert_eq!(restored.get("value"), doc.get("value"));
    }
}

// ========== PROPERTY 2: Storage Write/Read Roundtrip ==========

proptest! {
    #[test]
    fn prop_storage_write_read_roundtrip(data in prop::collection::vec(any::<u8>(), 1..10000)) {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.mlite");
        let mut storage = StorageEngine::open(&db_path).unwrap();

        // Write data
        let offset = storage.write_data(&data).unwrap();

        // Read back
        let read_data = storage.read_data(offset).unwrap();

        // Invariant: read data == written data
        assert_eq!(read_data, data);
    }
}

proptest! {
    #[test]
    fn prop_storage_multiple_writes_isolated(
        data1 in prop::collection::vec(any::<u8>(), 1..1000),
        data2 in prop::collection::vec(any::<u8>(), 1..1000),
        data3 in prop::collection::vec(any::<u8>(), 1..1000),
    ) {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.mlite");
        let mut storage = StorageEngine::open(&db_path).unwrap();

        // Write three blocks
        let offset1 = storage.write_data(&data1).unwrap();
        let offset2 = storage.write_data(&data2).unwrap();
        let offset3 = storage.write_data(&data3).unwrap();

        // Read all three back
        let read1 = storage.read_data(offset1).unwrap();
        let read2 = storage.read_data(offset2).unwrap();
        let read3 = storage.read_data(offset3).unwrap();

        // Invariant: each read matches its write (isolation)
        assert_eq!(read1, data1);
        assert_eq!(read2, data2);
        assert_eq!(read3, data3);
    }
}

// ========== PROPERTY 3: Query Operator Properties ==========

proptest! {
    #[test]
    fn prop_query_and_commutative(age1 in 0i64..100, age2 in 0i64..100, test_age in 0i64..100) {
        // Create test document
        let mut fields = HashMap::new();
        fields.insert("age".to_string(), json!(test_age));
        let doc = Document::new(DocumentId::Int(1), fields);

        // Query 1: $and with age > age1 AND age < age2
        let query1 = Query::from_json(&json!({
            "$and": [
                {"age": {"$gt": age1}},
                {"age": {"$lt": age2}}
            ]
        })).unwrap();

        // Query 2: same but reversed order
        let query2 = Query::from_json(&json!({
            "$and": [
                {"age": {"$lt": age2}},
                {"age": {"$gt": age1}}
            ]
        })).unwrap();

        // Invariant: AND is commutative - order doesn't matter
        assert_eq!(query1.matches(&doc), query2.matches(&doc));
    }
}

proptest! {
    #[test]
    fn prop_query_or_commutative(val1 in any::<i64>(), val2 in any::<i64>(), test_val in any::<i64>()) {
        let mut fields = HashMap::new();
        fields.insert("value".to_string(), json!(test_val));
        let doc = Document::new(DocumentId::Int(1), fields);

        // Query 1: value == val1 OR value == val2
        let query1 = Query::from_json(&json!({
            "$or": [
                {"value": val1},
                {"value": val2}
            ]
        })).unwrap();

        // Query 2: reversed order
        let query2 = Query::from_json(&json!({
            "$or": [
                {"value": val2},
                {"value": val1}
            ]
        })).unwrap();

        // Invariant: OR is commutative
        assert_eq!(query1.matches(&doc), query2.matches(&doc));
    }
}

// ========== PROPERTY 4: Document Field Operations ==========

proptest! {
    #[test]
    fn prop_document_set_get_consistency(
        field_name in "[a-z]{1,20}",
        value in any::<i64>(),
    ) {
        let mut doc = Document::new(DocumentId::Int(1), HashMap::new());

        // Set a field
        doc.set(field_name.clone(), json!(value));

        // Invariant: get returns what we set
        assert_eq!(doc.get(&field_name), Some(&json!(value)));
    }
}

proptest! {
    #[test]
    fn prop_document_remove_idempotent(
        field_name in "[a-z]{1,20}",
    ) {
        let mut fields = HashMap::new();
        fields.insert(field_name.clone(), json!(42));
        let mut doc = Document::new(DocumentId::Int(1), fields);

        // Remove once
        let removed1 = doc.remove(&field_name);
        assert!(removed1.is_some());

        // Remove again
        let removed2 = doc.remove(&field_name);

        // Invariant: second remove returns None (idempotent)
        assert!(removed2.is_none());
        assert!(!doc.contains(&field_name));
    }
}

// ========== PROPERTY 5: Collection Metadata ==========

proptest! {
    #[test]
    fn prop_collection_creation_idempotent(name in "[a-z]{1,20}") {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.mlite");
        let mut storage = StorageEngine::open(&db_path).unwrap();

        // Create collection
        storage.create_collection(&name).unwrap();

        // Try creating again (should fail but not panic)
        let result = storage.create_collection(&name);

        // Invariant: second create fails gracefully
        assert!(result.is_err());

        // Collection still exists
        assert!(storage.get_collection_meta(&name).is_some());
    }
}

proptest! {
    #[test]
    fn prop_list_collections_consistency(
        names in prop::collection::vec("[a-z]{1,15}", 1..10),
    ) {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.mlite");
        let mut storage = StorageEngine::open(&db_path).unwrap();

        // Create collections (dedup names)
        let mut unique_names: Vec<String> = names.clone();
        unique_names.sort();
        unique_names.dedup();

        for name in &unique_names {
            storage.create_collection(name).unwrap();
        }

        // List collections
        let mut listed = storage.list_collections();
        listed.sort();

        // Invariant: listed collections match created collections
        assert_eq!(listed, unique_names);
    }
}

// ========== PROPERTY 6: Numeric Comparison Operators ==========

proptest! {
    #[test]
    fn prop_query_gt_lte_inverse(value in any::<i64>(), threshold in any::<i64>()) {
        let mut fields = HashMap::new();
        fields.insert("value".to_string(), json!(value));
        let doc = Document::new(DocumentId::Int(1), fields);

        // Query 1: value > threshold
        let gt_query = Query::from_json(&json!({"value": {"$gt": threshold}})).unwrap();

        // Query 2: value <= threshold
        let lte_query = Query::from_json(&json!({"value": {"$lte": threshold}})).unwrap();

        // Invariant: $gt and $lte are inverses (exactly one should match, unless edge case)
        let gt_match = gt_query.matches(&doc);
        let lte_match = lte_query.matches(&doc);

        // One and only one should be true (unless value == threshold, then lte is true)
        if value == threshold {
            assert!(!gt_match && lte_match);
        } else {
            assert!(gt_match != lte_match); // XOR
        }
    }
}

proptest! {
    #[test]
    fn prop_query_in_membership(
        values in prop::collection::vec(any::<i64>(), 1..20),
        test_value in any::<i64>(),
    ) {
        let mut fields = HashMap::new();
        fields.insert("value".to_string(), json!(test_value));
        let doc = Document::new(DocumentId::Int(1), fields);

        // Query: value in values array
        let query = Query::from_json(&json!({
            "value": {"$in": values}
        })).unwrap();

        let matches = query.matches(&doc);
        let expected = values.contains(&test_value);

        // Invariant: $in matches iff value is in array
        assert_eq!(matches, expected);
    }
}
